//! Independent native device: one MLS store, a platform-held vault key, and an
//! exact route allowlist. This crate never opens the multi-account demo workspace.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_client::{ClientStore, application::*};
use croaker_crypto::{Device, DeviceBinding};
#[cfg(not(target_arch = "wasm32"))]
use croaker_crypto::{open_record, seal_record};
use croaker_protocol::{enrollment::*, *};
#[cfg(not(target_arch = "wasm32"))]
use fs2::FileExt;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    fs::{File, OpenOptions},
    path::Path,
    sync::Arc,
};
use zeroize::Zeroizing;
mod account;
mod backups;
#[cfg(target_arch = "wasm32")]
pub mod browser;
mod groups;
#[cfg(not(target_arch = "wasm32"))]
pub mod ios;
mod media;
mod messaging;
#[cfg(not(target_arch = "wasm32"))]
pub mod oauth;
#[cfg(target_arch = "wasm32")]
#[path = "browser_oauth.rs"]
pub mod oauth;

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>>;
    fn set(&self, name: &str, secret: &[u8]) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
}
#[derive(Serialize)]
pub struct DeviceResponse {
    pub status: u16,
    pub body: Value,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    relay: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contact {
    version: u16,
    name: String,
    binding: DeviceBinding,
    package: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Chat {
    id: ConversationId,
    title: String,
    kind: String,
    owner: DeviceId,
    members: Vec<DeviceId>,
    #[serde(default)]
    created: bool,
}
#[derive(Serialize, Deserialize)]
struct Private {
    binding: DeviceBinding,
    relay: String,
    token: String,
    account_id: String,
    name: String,
    role: Role,
    enrollment: EnrollmentInput,
    approved: bool,
    #[serde(default)]
    locked: bool,
    contacts: BTreeMap<DeviceId, Contact>,
    chats: BTreeMap<ConversationId, Chat>,
    #[serde(default)]
    read_receipts: bool,
    #[serde(default)]
    approvals: Vec<croaker_client::ApprovedAddition>,
    #[serde(default)]
    lifecycles: Vec<croaker_client::lifecycle::ApprovedLifecycle>,
    #[serde(default)]
    group_decisions: Vec<groups::GroupDecision>,
}
impl Drop for Private {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.token.zeroize();
    }
}
pub struct DeviceRuntime {
    #[cfg(not(target_arch = "wasm32"))]
    directory: PathBuf,
    #[cfg(not(target_arch = "wasm32"))]
    _lock: File,
    key: Zeroizing<[u8; 32]>,
    connection: Option<Connection>,
    private: Option<Private>,
    client: Option<ClientStore>,
    http: reqwest::Client,
    login: Option<oauth::PendingLogin>,
    login_config: Option<oauth::LoginConfig>,
    access: Option<Zeroizing<String>>,
    account: Option<Value>,
    online: bool,
    typing: BTreeMap<(ConversationId, DeviceId), u64>,
    locked: bool,
    persistence_failed: std::cell::Cell<bool>,
}
#[cfg(not(target_arch = "wasm32"))]
fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
#[cfg(not(target_arch = "wasm32"))]
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temp = path.with_extension("pending");
    let mut file = private_file(&temp)?;
    file.set_len(0)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(temp, path)?;
    File::open(path.parent().context("Missing directory")?)?.sync_all()?;
    Ok(())
}
fn random_token() -> String {
    let mut bytes = Zeroizing::new([0; 32]);
    OsRng.fill_bytes(bytes.as_mut());
    hex::encode(*bytes)
}
#[cfg(not(target_arch = "wasm32"))]
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[cfg(target_arch = "wasm32")]
fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}
fn checked_url(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value)?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Use a relay URL without credentials, query, or fragment."
    );
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
        "Relay requires HTTPS; HTTP is allowed only on loopback for tests."
    );
    ensure!(url.host_str().is_some(), "Missing relay host");
    Ok(url)
}
impl DeviceRuntime {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(directory: PathBuf, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let directory = directory.canonicalize()?;
        let lock = private_file(&directory.join("device.lock"))?;
        lock.try_lock_exclusive()
            .context("This device store is already open in another process.")?;
        let installation_path = directory.join("installation.id");
        let installation = if installation_path.exists() {
            std::fs::read_to_string(&installation_path)?
        } else {
            let id = MessageId::new().to_string();
            atomic_write(&installation_path, id.as_bytes())?;
            id
        };
        let _: MessageId = installation
            .parse()
            .context("Invalid installation identity")?;
        let secret_name = format!("vault/{installation}");
        let saved = secrets.get(&secret_name)?;
        let key = if let Some(saved) = saved {
            Zeroizing::new(saved.as_slice().try_into().context("Invalid vault key")?)
        } else {
            ensure!(
                !directory.join("client.sqlite").exists()
                    && !directory.join("device.vault").exists(),
                "The Keychain vault key is missing. Existing encrypted data was preserved."
            );
            let mut key = Zeroizing::new([0; 32]);
            OsRng.fill_bytes(key.as_mut());
            secrets.set(&secret_name, key.as_slice())?;
            key
        };
        let connection = if directory.join("connection.json").exists() {
            Some(serde_json::from_slice::<Connection>(&std::fs::read(
                directory.join("connection.json"),
            )?)?)
        } else {
            None
        };
        if let Some(c) = &connection {
            checked_url(&c.relay)?;
        }
        let private = if directory.join("device.vault").exists() {
            Some(serde_json::from_slice::<Private>(&open_record(
                &std::fs::read(directory.join("device.vault"))?,
                &key,
                b"cirra-native-profile-v1",
            )?)?)
        } else {
            None
        };
        if let Some(profile) = &private {
            ensure!(
                connection
                    .as_ref()
                    .is_some_and(|c| c.relay == profile.relay),
                "Relay configuration differs from the encrypted device enrollment. Existing data was preserved."
            );
        }
        let client = if directory.join("client.sqlite").exists() {
            Some(ClientStore::open(
                directory.join("client.sqlite"),
                Zeroizing::new(*key),
            )?)
        } else {
            None
        };
        ensure!(
            private.is_none() || client.is_some(),
            "The device state is missing. Existing data was preserved."
        );
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let locked = private.as_ref().is_some_and(|p| p.locked);
        Ok(Self {
            directory,
            _lock: lock,
            key,
            connection,
            private,
            client,
            http,
            login: None,
            login_config: None,
            access: None,
            account: None,
            online: true,
            typing: BTreeMap::new(),
            locked,
            persistence_failed: std::cell::Cell::new(false),
        })
    }
    fn ensure_healthy(&self) -> Result<()> {
        ensure!(
            !self.persistence_failed.get(),
            "Device state could not be saved. Restart after correcting storage access; queued ciphertext was preserved."
        );
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn save(&self) -> Result<()> {
        self.ensure_healthy()?;
        let result = (|| -> Result<()> {
            if let Some(private) = &self.private {
                let bytes = Zeroizing::new(serde_json::to_vec(private)?);
                atomic_write(
                    &self.directory.join("device.vault"),
                    &seal_record(&bytes, &self.key, b"cirra-native-profile-v1")?,
                )?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.persistence_failed.set(true);
        }
        result
    }
    fn profile(&self) -> Result<&Private> {
        self.private.as_ref().context("Enroll this device first.")
    }
    fn client(&mut self) -> Result<&mut ClientStore> {
        self.client.as_mut().context("Enroll this device first.")
    }
    async fn remote(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        account: bool,
    ) -> Result<Value> {
        ensure!(self.online, "The device is offline.");
        let relay = &self
            .connection
            .as_ref()
            .context("Configure a relay first.")?
            .relay;
        ensure!(
            path.starts_with("/v1/") && !path.contains("..") && !path.contains('#'),
            "Invalid relay route"
        );
        let url = format!("{}{path}", relay.trim_end_matches('/'));
        let mut request = self
            .http
            .request(method.parse()?, url)
            .timeout(std::time::Duration::from_secs(15));
        if account {
            request = request.bearer_auth(
                self.access
                    .as_ref()
                    .context("Sign in again to manage account enrollment.")?
                    .as_str(),
            );
        } else {
            request = request.bearer_auth(&self.profile()?.token);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        self.checkpoint().await?;
        let response = request.send().await.context("Could not reach the relay.")?;
        let status = response.status();
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= 4 * 1024 * 1024),
            "Relay response is too large"
        );
        let bytes = bounded_response(response, 4 * 1024 * 1024).await?;
        ensure!(status.is_success(), "Relay request failed ({status}).");
        if bytes.is_empty() {
            Ok(json!({}))
        } else {
            Ok(serde_json::from_slice(&bytes)?)
        }
    }
    pub fn is_authorization_url(&self, url: &str) -> bool {
        self.login
            .as_ref()
            .is_some_and(|login| login.authorize_url == url)
    }
    pub async fn request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> DeviceResponse {
        match self
            .dispatch(method, path, body.unwrap_or_else(|| json!({})))
            .await
        {
            Ok(body) => DeviceResponse { status: 200, body },
            Err(error) => DeviceResponse {
                status: 400,
                body: json!({"error":error.to_string()}),
            },
        }
    }
    async fn dispatch(&mut self, method: &str, path: &str, body: Value) -> Result<Value> {
        ensure!(
            matches!(method, "GET" | "POST")
                && path.len() < 2048
                && !path.contains(['%', '\\', '#'])
                && !path.contains("..")
                && !path.bytes().any(|b| b.is_ascii_control()),
            "Unsupported client route"
        );
        ensure!(
            serde_json::to_vec(&body)?.len() <= 40 * 1024 * 1024,
            "Request is too large"
        );
        self.ensure_healthy()?;
        match (method, path) {
            ("GET", "/desktop/status") => {
                return Ok(
                    json!({"configured":self.connection.is_some(),"relay":self.connection.as_ref().map(|c|&c.relay),"enrolled":self.private.is_some(),"approved":self.private.as_ref().is_some_and(|p|p.approved),"fingerprint":self.private.as_ref().map(|p|p.binding.fingerprint()),"account_id":self.private.as_ref().map(|p|&p.account_id)}),
                );
            }
            ("POST", "/desktop/configure") => {
                ensure!(
                    self.private.is_none(),
                    "An enrolled installation cannot switch relay."
                );
                let relay = body["relay"]
                    .as_str()
                    .context("Enter the relay URL.")?
                    .trim_end_matches('/')
                    .to_owned();
                checked_url(&relay)?;
                let connection = Connection { relay };
                #[cfg(not(target_arch = "wasm32"))]
                atomic_write(
                    &self.directory.join("connection.json"),
                    &serde_json::to_vec(&connection)?,
                )?;
                self.connection = Some(connection);
                return Ok(json!({}));
            }
            ("POST", "/desktop/auth/start") => return self.begin_login().await,
            ("GET", "/local-auth/me") => return self.session().await,
            ("POST", "/local-auth/logout") => {
                self.login = None;
                self.login_config = None;
                self.access = None;
                self.account = None;
                self.locked = true;
                if let Some(profile) = &mut self.private {
                    profile.locked = true;
                    self.save()?;
                }
                return Ok(json!({"redirect":null}));
            }
            _ => {}
        }
        ensure!(!self.locked, "Sign in to unlock this device.");
        if path == "/desktop/enrollment" && method == "POST" {
            return self.enroll().await;
        }
        ensure!(
            self.profile()?.approved,
            "This device requires fingerprint approval."
        );
        if let Some(result) = self.groups_management(method, path, &body).await? {
            return Ok(result);
        }
        if let Some(result) = self.management(method, path, &body).await? {
            return Ok(result);
        }
        if method == "GET" && path.starts_with("/local-api/state?profile=") {
            ensure!(
                path.strip_prefix("/local-api/state?profile=")
                    == Some(self.profile()?.binding.device_id.to_string().as_str()),
                "Unknown device"
            );
            return self.snapshot().await;
        }
        let prefix = format!("/local-api/{}/", self.profile()?.binding.device_id);
        if let Some(route) = path.strip_prefix(&prefix) {
            return self.messaging_request(method, route, body).await;
        }
        bail!("Unsupported client route")
    }
}

#[cfg(test)]
mod tests;

async fn bounded_response(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    use futures_util::StreamExt;
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "Response exceeds its expected size"
    );
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        ensure!(
            bytes.len() + chunk.len() <= limit,
            "Response exceeds its expected size"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
#[cfg(not(target_arch = "wasm32"))]
impl DeviceRuntime {
    // Retain an exclusive borrow across await. SQLite is Send but not Sync;
    // a shared runtime borrow would make native request futures non-Send.
    async fn checkpoint(&mut self) -> Result<()> {
        self.ensure_healthy()
    }
}

impl DeviceRuntime {
    fn client_path(&self) -> PathBuf {
        #[cfg(target_arch = "wasm32")]
        {
            PathBuf::from(":memory:")
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.directory.join("client.sqlite")
        }
    }
}
