//! Native iOS authentication entry points. Only the platform shell invokes these;
//! authorization callbacks and OAuth credentials never enter the renderer bridge.
use super::*;
impl DeviceRuntime {
    pub async fn begin_platform_login(&mut self) -> Result<oauth::PlatformLogin> {
        self.login = None;
        let config = self.load_login_config().await?;
        oauth::PlatformLogin::begin(config)
    }
    pub async fn complete_platform_login(&mut self, tokens: oauth::OAuthTokens) -> Result<()> {
        self.accept_access(tokens.access_token).await
    }
}
