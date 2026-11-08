use super::*;
use crate::auth::AuthService;
use croaker_protocol::enrollment::{EnrollmentInput, IdentityBinding};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
struct Candidate {
    owner: String,
    account_id: String,
    alias: String,
    input: EnrollmentInput,
    token: String,
    vault_key: String,
}
impl Workspace {
    async fn request_device(
        &mut self,
        owner: &str,
        account: &str,
        id: MessageId,
        label: String,
    ) -> Result<()> {
        let path = self.directory.join(format!("enrollment-{id}.local.json"));
        let candidate = if path.exists() {
            let bytes = Zeroizing::new(std::fs::read(&path)?);
            let candidate: Candidate = serde_json::from_slice(&bytes)?;
            ensure!(
                candidate.owner == owner
                    && candidate.account_id == account
                    && candidate.input.label == label,
                "Conflicting device request."
            );
            candidate
        } else {
            let principal = self.profile(owner)?.principal;
            let device_id = DeviceId::new();
            let device = Device::new(principal.tenant_id, device_id)?;
            let binding = device.binding().clone();
            let alias = format!("device-{device_id}");
            let vault = random_key();
            let token = hex::encode(*random_key());
            let mut input = EnrollmentInput {
                id,
                binding: IdentityBinding {
                    tenant_id: binding.tenant_id,
                    device_id: binding.device_id,
                    signature_key: binding.signature_key,
                },
                label,
                token_hash: hex::encode(Sha256::digest(token.as_bytes())),
                proof: vec![],
            };
            let mut client = ClientStore::create(
                self.directory.join(format!("{alias}.sqlite")),
                device,
                Zeroizing::new(*vault),
            )?;
            input.proof = client.update(|device| device.sign_enrollment(&input, account))?;
            let candidate = Candidate {
                owner: owner.into(),
                account_id: account.into(),
                alias,
                input,
                token,
                vault_key: hex::encode(*vault),
            };
            let bytes = Zeroizing::new(serde_json::to_vec(&candidate)?);
            write_private(&path, &bytes)?;
            candidate
        };
        self.relay
            .request_enrollment(account, candidate.input)
            .await?;
        Ok(())
    }
    async fn activate_device(
        &mut self,
        owner: &str,
        account: &str,
        id: MessageId,
    ) -> Result<String> {
        let bytes = Zeroizing::new(
            std::fs::read(self.directory.join(format!("enrollment-{id}.local.json")))
                .context("This device's private keys are not on this machine.")?,
        );
        let candidate: Candidate = serde_json::from_slice(&bytes)?;
        ensure!(
            candidate.owner == owner && candidate.account_id == account,
            "Device belongs to another account."
        );
        let record = self.relay.account(account).await?;
        ensure!(
            record
                .devices
                .iter()
                .any(|d| d.binding == candidate.input.binding),
            "This device has not been approved."
        );
        let principal = self.relay.authenticate(&candidate.token).await?;
        ensure!(
            principal.device_id == candidate.input.binding.device_id
                && principal.tenant_id == record.grant.tenant_id,
            "Device credential does not match."
        );
        if self
            .manifest
            .profiles
            .iter()
            .any(|p| p.alias == candidate.alias)
        {
            return Ok(candidate.alias);
        }
        let key = Zeroizing::new(hex::decode(&candidate.vault_key)?);
        let key = Zeroizing::new(
            <[u8; 32]>::try_from(key.as_slice()).context("Invalid local device vault")?,
        );
        let client = ClientStore::open(
            self.directory.join(format!("{}.sqlite", candidate.alias)),
            key,
        )?;
        self.manifest.profiles.push(Profile {
            alias: candidate.alias.clone(),
            name: record.grant.name,
            title: candidate.input.label,
            principal,
            token: candidate.token,
            vault_key: candidate.vault_key,
            binding: DeviceBinding {
                tenant_id: candidate.input.binding.tenant_id,
                device_id: candidate.input.binding.device_id,
                signature_key: candidate.input.binding.signature_key,
            },
            owner: Some(owner.into()),
        });
        if let Err(error) = self.save() {
            self.manifest.profiles.pop();
            return Err(error);
        }
        self.clients.insert(candidate.alias.clone(), client);
        self.online.insert(candidate.alias.clone(), true);
        Ok(candidate.alias)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestDevice {
    id: MessageId,
    label: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    account_id: String,
    id: MessageId,
    fingerprint: String,
    approve: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Activate {
    id: MessageId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UseDevice {
    alias: String,
}
fn signin() -> LocalError {
    croaker_relay::StoreError::Unauthorized.into()
}
async fn request(
    State(auth): State<Arc<AuthService>>,
    headers: axum::http::HeaderMap,
    Json(input): Json<RequestDevice>,
) -> std::result::Result<StatusCode, LocalError> {
    let session = auth.session(&headers).await.map_err(|_| signin())?;
    auth.workspace
        .lock()
        .await
        .request_device(&session.owner, &session.account_id, input.id, input.label)
        .await?;
    Ok(StatusCode::ACCEPTED)
}
async fn inspect(
    State(auth): State<Arc<AuthService>>,
    headers: axum::http::HeaderMap,
    Query(input): Query<HashMap<String, String>>,
) -> std::result::Result<Json<serde_json::Value>, LocalError> {
    let session = auth.session(&headers).await.map_err(|_| signin())?;
    let workspace = auth.workspace.lock().await;
    let profile = workspace.profile(&session.active_alias)?;
    let actor = workspace.relay.authenticate(&profile.token).await?;
    let account = workspace
        .relay
        .account(
            input
                .get("account")
                .map(String::as_str)
                .unwrap_or(&session.account_id),
        )
        .await?;
    account.can_approve(&actor)?;
    Ok(Json(account.public()))
}
async fn decide(
    State(auth): State<Arc<AuthService>>,
    headers: axum::http::HeaderMap,
    Json(input): Json<Decision>,
) -> std::result::Result<StatusCode, LocalError> {
    let session = auth.session(&headers).await.map_err(|_| signin())?;
    let workspace = auth.workspace.lock().await;
    let profile = workspace.profile(&session.active_alias)?;
    let actor = workspace.relay.authenticate(&profile.token).await?;
    workspace
        .relay
        .decide_enrollment(
            &actor,
            &input.account_id,
            input.id,
            &input.fingerprint,
            input.approve,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn activate(
    State(auth): State<Arc<AuthService>>,
    headers: axum::http::HeaderMap,
    Json(input): Json<Activate>,
) -> std::result::Result<Json<serde_json::Value>, LocalError> {
    let session = auth.session(&headers).await.map_err(|_| signin())?;
    let alias = auth
        .workspace
        .lock()
        .await
        .activate_device(&session.owner, &session.account_id, input.id)
        .await?;
    auth.set_active(&headers, alias.clone()).await?;
    Ok(Json(serde_json::json!({"profile":alias})))
}
async fn select(
    State(auth): State<Arc<AuthService>>,
    headers: axum::http::HeaderMap,
    Json(input): Json<UseDevice>,
) -> std::result::Result<StatusCode, LocalError> {
    let session = auth.session(&headers).await.map_err(|_| signin())?;
    let workspace = auth.workspace.lock().await;
    let profile = workspace.profile(&input.alias)?;
    if profile.owner.as_deref().unwrap_or(&profile.alias) != session.owner {
        return Ok(StatusCode::FORBIDDEN);
    }
    workspace.relay.authenticate(&profile.token).await?;
    drop(workspace);
    auth.set_active(&headers, input.alias).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revoke {
    device_id: DeviceId,
}
async fn revoke(
    State(auth): State<Arc<AuthService>>,
    headers: axum::http::HeaderMap,
    Json(input): Json<Revoke>,
) -> std::result::Result<StatusCode, LocalError> {
    let session = auth.session(&headers).await.map_err(|_| signin())?;
    let workspace = auth.workspace.lock().await;
    let profile = workspace.profile(&session.active_alias)?;
    let actor = workspace.relay.authenticate(&profile.token).await?;
    workspace
        .relay
        .revoke_owned_device(&actor, &session.account_id, input.device_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) fn routes() -> Router<Arc<AuthService>> {
    Router::new()
        .route("/local-auth/devices/revoke", post(revoke))
        .route("/local-auth/devices/request", post(request))
        .route("/local-auth/devices/inspect", get(inspect))
        .route("/local-auth/devices/decision", post(decide))
        .route("/local-auth/devices/activate", post(activate))
        .route("/local-auth/devices/select", post(select))
}
