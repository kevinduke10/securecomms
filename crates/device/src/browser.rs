//! Browser adapter for the existing Rust device engine. The host commits the
//! complete encrypted snapshot before any relay mutation can leave the worker.
use super::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_namespace = globalThis, js_name = cirraPersist)]
    fn persist(snapshot: &str) -> std::result::Result<js_sys::Promise, JsValue>;
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u16,
    connection: Option<Connection>,
    private: Option<Private>,
    database: Option<String>,
}
fn failure(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
#[wasm_bindgen]
pub struct BrowserDevice {
    runtime: DeviceRuntime,
}
#[wasm_bindgen]
impl BrowserDevice {
    #[wasm_bindgen(constructor)]
    pub fn new(key: &[u8], snapshot: &str) -> std::result::Result<BrowserDevice, JsValue> {
        Self::load(key, snapshot).map_err(failure)
    }
    pub async fn request(
        &mut self,
        method: &str,
        path: &str,
        body: &str,
    ) -> std::result::Result<String, JsValue> {
        let body = serde_json::from_str(body).map_err(failure)?;
        let response = self.runtime.request(method, path, Some(body)).await;
        self.runtime.checkpoint().await.map_err(failure)?;
        serde_json::to_string(&response).map_err(failure)
    }
    pub async fn sign_in(&mut self, access: String) -> std::result::Result<(), JsValue> {
        self.runtime
            .accept_access(Zeroizing::new(access))
            .await
            .map_err(failure)?;
        self.runtime.checkpoint().await.map_err(failure)
    }
}
impl BrowserDevice {
    fn load(key: &[u8], snapshot: &str) -> Result<Self> {
        let key = Zeroizing::new(key.try_into().context("Invalid browser vault key")?);
        let saved: Snapshot = if snapshot.is_empty() {
            Snapshot {
                version: 1,
                connection: None,
                private: None,
                database: None,
            }
        } else {
            serde_json::from_str(snapshot)?
        };
        ensure!(saved.version == 1, "Unsupported browser store version");
        if let Some(connection) = &saved.connection {
            checked_url(&connection.relay)?;
        }
        if let Some(profile) = &saved.private {
            ensure!(
                saved
                    .connection
                    .as_ref()
                    .is_some_and(|c| c.relay == profile.relay),
                "Relay differs from the enrolled device; existing data preserved"
            );
            ensure!(
                saved.database.is_some(),
                "Device database missing; existing data preserved"
            );
        }
        let client = saved
            .database
            .map(|s| -> Result<_> {
                Ok(ClientStore::browser_open(
                    &BASE64_STANDARD.decode(s)?,
                    Zeroizing::new(*key),
                )?)
            })
            .transpose()?;
        let locked = saved.private.as_ref().is_some_and(|p| p.locked);
        Ok(Self {
            runtime: DeviceRuntime {
                key,
                connection: saved.connection,
                private: saved.private,
                client,
                http: reqwest::Client::new(),
                login: None,
                login_config: None,
                access: None,
                account: None,
                online: true,
                typing: BTreeMap::new(),
                locked,
                persistence_failed: std::cell::Cell::new(false),
            },
        })
    }
}
impl DeviceRuntime {
    pub(super) fn save(&self) -> Result<()> {
        self.ensure_healthy()
    }
    pub(super) async fn checkpoint(&mut self) -> Result<()> {
        self.ensure_healthy()?;
        let result = async {
            let database = self
                .client
                .as_ref()
                .map(|c| c.browser_export().map(|b| BASE64_STANDARD.encode(b)))
                .transpose()?;
            let snapshot = Zeroizing::new(serde_json::to_string(&serde_json::json!({
                "version":1,"connection":self.connection,"private":self.private,"database":database
            }))?);
            let promise = persist(&snapshot).map_err(|_| {
                anyhow::anyhow!("Browser storage unavailable; reload after fixing storage")
            })?;
            JsFuture::from(promise).await.map_err(|_| {
                anyhow::anyhow!("Browser state could not be saved. Reload to recover the committed device state.")
            })?;
            Ok(())
        }
        .await;
        if result.is_err() {
            self.persistence_failed.set(true);
        }
        result
    }
}
