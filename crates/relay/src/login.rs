//! Cognito access-token verification. Login never returns messaging credentials.
use crate::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, serde::Serialize)]
pub struct LoginConfig {
    pub issuer: String,
    pub client_id: String,
    pub domain: String,
    pub scope: String,
}
impl LoginConfig {
    pub fn from_env() -> std::result::Result<Self, anyhow::Error> {
        let config = Self {
            issuer: std::env::var("CROAKER_COGNITO_ISSUER")?,
            client_id: std::env::var("CROAKER_COGNITO_CLIENT_ID")?,
            domain: std::env::var("CROAKER_COGNITO_DOMAIN")?,
            scope: "openid croaker/access".into(),
        };
        config
            .validate()
            .map_err(|_| anyhow::anyhow!("invalid Cognito configuration"))?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        for value in [&self.issuer, &self.domain] {
            let url =
                reqwest::Url::parse(value).map_err(|_| StoreError::Invalid("invalid login URL"))?;
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || value.ends_with('/')
            {
                return Err(StoreError::Invalid("login requires canonical HTTPS URLs"));
            }
        }
        if self.client_id.is_empty() || self.client_id.len() > 128 {
            return Err(StoreError::Invalid("invalid login client"));
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub client_id: String,
    pub token_use: String,
    pub exp: u64,
    pub iat: u64,
    pub scope: String,
}
struct Keys {
    set: JwkSet,
    fetched: u64,
    attempted: u64,
}
pub struct LoginVerifier {
    pub config: LoginConfig,
    http: reqwest::Client,
    keys: Mutex<Keys>,
}
impl LoginVerifier {
    pub fn new(config: LoginConfig) -> Result<Self> {
        config.validate()?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|_| StoreError::Unavailable)?;
        Ok(Self {
            config,
            http,
            keys: Mutex::new(Keys {
                set: JwkSet { keys: vec![] },
                fetched: 0,
                attempted: 0,
            }),
        })
    }
    /// Explicit public-key pinning also supports offline verifier tests.
    pub async fn with_keys(config: LoginConfig, set: JwkSet) -> Result<Self> {
        let verifier = Self::new(config)?;
        {
            let mut keys = verifier.keys.lock().await;
            keys.set = set;
            keys.fetched = now();
            keys.attempted = now();
        }
        Ok(verifier)
    }
    pub async fn verify(&self, token: &str) -> Result<Claims> {
        if token.len() > 16 * 1024 {
            return Err(StoreError::Unauthorized);
        }
        let header = decode_header(token).map_err(|_| StoreError::Unauthorized)?;
        if header.alg != Algorithm::RS256 {
            return Err(StoreError::Unauthorized);
        }
        let kid = header
            .kid
            .filter(|kid| !kid.is_empty() && kid.len() <= 128)
            .ok_or(StoreError::Unauthorized)?;
        let mut keys = self.keys.lock().await;
        let timestamp = now();
        if (keys.fetched + 3600 < timestamp || keys.set.find(&kid).is_none())
            && keys.attempted + 30 < timestamp
        {
            keys.attempted = timestamp;
            let response = self
                .http
                .get(format!("{}/.well-known/jwks.json", self.config.issuer))
                .send()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            if !response.status().is_success()
                || response.content_length().is_some_and(|n| n > 64 * 1024)
            {
                return Err(StoreError::Unavailable);
            }
            // Stream-bounded read; an untrusted response cannot bypass Content-Length.
            let mut response = response;
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| StoreError::Unavailable)?
            {
                if bytes.len() + chunk.len() > 64 * 1024 {
                    return Err(StoreError::Unavailable);
                }
                bytes.extend_from_slice(&chunk);
            }
            let set: JwkSet =
                serde_json::from_slice(&bytes).map_err(|_| StoreError::Unavailable)?;
            if set.keys.len() > 16 {
                return Err(StoreError::Unavailable);
            }
            keys.set = set;
            keys.fetched = timestamp;
        }
        if keys.fetched + 3600 < timestamp {
            return Err(StoreError::Unavailable);
        }
        let jwk = keys.set.find(&kid).ok_or(StoreError::Unauthorized)?;
        if jwk.common.key_algorithm != Some(jsonwebtoken::jwk::KeyAlgorithm::RS256)
            || jwk.common.public_key_use != Some(jsonwebtoken::jwk::PublicKeyUse::Signature)
        {
            return Err(StoreError::Unauthorized);
        }
        let key = DecodingKey::from_jwk(jwk).map_err(|_| StoreError::Unauthorized)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.config.issuer]);
        validation.validate_aud = false;
        validation.leeway = 30;
        validation.set_required_spec_claims(&["exp", "iss", "sub", "iat"]);
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| StoreError::Unauthorized)?
            .claims;
        if claims.token_use != "access"
            || claims.client_id != self.config.client_id
            || claims.iss != self.config.issuer
            || claims.sub.is_empty()
            || claims.sub.len() > 128
            || claims.iat > timestamp + 30
            || claims.exp <= claims.iat
            || claims.exp.saturating_sub(claims.iat) > 3600
            || !claims
                .scope
                .split_whitespace()
                .any(|scope| scope == "croaker/access")
        {
            return Err(StoreError::Unauthorized);
        }
        Ok(claims)
    }
}
#[derive(Clone)]
struct LoginState {
    store: Arc<dyn Store>,
    verifier: Arc<LoginVerifier>,
}
fn bearer(headers: &HeaderMap) -> Result<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StoreError::Unauthorized)
}
async fn account_for(headers: &HeaderMap, state: &LoginState) -> Result<identity::AccountRecord> {
    let claims = state.verifier.verify(bearer(headers)?).await?;
    state
        .store
        .account(&identity::identity_id(&claims.iss, &claims.sub))
        .await
}
async fn account(
    State(state): State<LoginState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(account_for(&headers, &state).await?.public()))
}
async fn request(
    State(state): State<LoginState>,
    headers: HeaderMap,
    Json(input): Json<croaker_protocol::enrollment::EnrollmentInput>,
) -> Result<axum::http::StatusCode> {
    let account = account_for(&headers, &state).await?;
    state
        .store
        .request_enrollment(&account.grant.id(), input)
        .await?;
    Ok(axum::http::StatusCode::ACCEPTED)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    fingerprint: String,
    approve: bool,
}
async fn inspect(
    State(state): State<LoginState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.store.authenticate(bearer(&headers)?).await?;
    let account = state.store.account(&id).await?;
    account.can_approve(&actor)?;
    Ok(Json(account.public()))
}
async fn decide(
    State(state): State<LoginState>,
    headers: HeaderMap,
    Path((account, id)): Path<(String, MessageId)>,
    Json(input): Json<Decision>,
) -> Result<axum::http::StatusCode> {
    let actor = state.store.authenticate(bearer(&headers)?).await?;
    state
        .store
        .decide_enrollment(&actor, &account, id, &input.fingerprint, input.approve)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
async fn revoke_owned(
    State(state): State<LoginState>,
    headers: HeaderMap,
    Path((account, device)): Path<(String, DeviceId)>,
) -> Result<axum::http::StatusCode> {
    let actor = state.store.authenticate(bearer(&headers)?).await?;
    state
        .store
        .revoke_owned_device(&actor, &account, device)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
pub fn routes(store: Arc<dyn Store>, verifier: Arc<LoginVerifier>) -> Router {
    let config = verifier.config.clone();
    Router::new()
        .route(
            "/v1/enrollments/{account}/devices/{device}/revoke",
            post(revoke_owned),
        )
        .route(
            "/v1/login/config",
            get(move || {
                let config = config.clone();
                async move { Json(config) }
            }),
        )
        .route("/v1/account", get(account))
        .route("/v1/account/enrollments", post(request))
        .route("/v1/enrollments/{account}", get(inspect))
        .route("/v1/enrollments/{account}/{id}/decision", post(decide))
        .layer(axum::extract::DefaultBodyLimit::max(8 * 1024))
        .with_state(LoginState { store, verifier })
}
