//! Browser authorization is owned by the worker's PKCE redirect adapter.
use super::*;
#[derive(Clone, Deserialize)]
pub struct LoginConfig {
    pub issuer: String,
    pub client_id: String,
    pub domain: String,
    pub scope: String,
}
pub struct PendingLogin {
    pub authorize_url: String,
}
pub struct OAuthTokens {
    pub access_token: Zeroizing<String>,
}
pub async fn begin(_: LoginConfig) -> Result<PendingLogin> {
    bail!("Use browser sign-in")
}
impl PendingLogin {
    pub async fn try_code(&mut self) -> Result<Option<OAuthTokens>> {
        Ok(None)
    }
}
