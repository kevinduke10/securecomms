//! Native authorization-code + S256 PKCE. No token or verifier is renderer data.
use anyhow::{Result, bail, ensure};
use axum::{
    Router,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use reqwest::Url;
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, Notify, oneshot},
    task::JoinHandle,
    time::Instant,
};
use zeroize::Zeroizing;

pub const CALLBACK_URL: &str = "http://localhost:38317/callback";
const LIFETIME: Duration = Duration::from_secs(300);

#[derive(Clone, Deserialize)]
pub struct LoginConfig {
    pub issuer: String,
    pub client_id: String,
    pub domain: String,
    pub scope: String,
}

fn domain(config: &LoginConfig) -> Result<Url> {
    ensure!(
        !config
            .domain
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control()),
        "Invalid login domain"
    );
    let issuer = Url::parse(&config.issuer).map_err(|_| anyhow::anyhow!("Invalid login issuer"))?;
    let issuer_loopback = matches!(issuer.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    ensure!(
        (issuer.scheme() == "https" || issuer.scheme() == "http" && issuer_loopback)
            && issuer.host_str().is_some()
            && issuer.username().is_empty()
            && issuer.password().is_none()
            && issuer.query().is_none()
            && issuer.fragment().is_none()
            && !config
                .issuer
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control()),
        "Invalid login issuer"
    );
    let url = Url::parse(&config.domain).map_err(|_| anyhow::anyhow!("Invalid login domain"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "Login requires HTTPS"
    );
    ensure!(
        url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "Login domain must be an origin"
    );
    ensure!(
        !config.client_id.is_empty()
            && config.client_id.len() <= 128
            && config
                .client_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            && !config.scope.is_empty()
            && config.scope.split(' ').all(|scope| !scope.is_empty()
                && scope.bytes().all(|byte| byte == 0x21
                    || (0x23..=0x5b).contains(&byte)
                    || (0x5d..=0x7e).contains(&byte)))
            && config.scope.len() <= 2048,
        "Invalid login client configuration"
    );
    Ok(url)
}

pub fn is_authorization_url(config: &LoginConfig, candidate: &str) -> bool {
    let Ok(origin) = domain(config) else {
        return false;
    };
    let Ok(url) = Url::parse(candidate) else {
        return false;
    };
    url.origin() == origin.origin()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/oauth2/authorize"
        && url.fragment().is_none()
}

fn secret() -> Zeroizing<String> {
    let mut random = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(&mut *random);
    Zeroizing::new(URL_SAFE_NO_PAD.encode(*random))
}

fn deserialize_secret<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Zeroizing<String>, D::Error> {
    String::deserialize(deserializer).map(Zeroizing::new)
}

fn deserialize_optional_secret<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Zeroizing<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(|value| value.map(Zeroizing::new))
}

// Deliberately neither Serialize nor Debug.
#[derive(Deserialize)]
pub struct OAuthTokens {
    #[serde(deserialize_with = "deserialize_secret")]
    pub access_token: Zeroizing<String>,
    #[serde(default, deserialize_with = "deserialize_optional_secret")]
    pub refresh_token: Option<Zeroizing<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_secret")]
    pub id_token: Option<Zeroizing<String>>,
    pub expires_in: u64,
    token_type: String,
}

type CallbackResult = std::result::Result<Zeroizing<String>, ()>;
struct Flow {
    state: Zeroizing<String>,
    result: Mutex<Option<oneshot::Sender<CallbackResult>>>,
    shutdown: Notify,
}

fn callback_code(
    headers: &HeaderMap,
    raw: Option<&str>,
    expected_state: &str,
) -> Result<CallbackResult> {
    ensure!(
        headers.get_all("host").iter().count() == 1
            && headers.get("host").and_then(|value| value.to_str().ok()) == Some("localhost:38317"),
        "Invalid callback host"
    );
    let raw = raw.ok_or_else(|| anyhow::anyhow!("Missing callback parameters"))?;
    ensure!(raw.len() <= 8192, "Callback is too large");
    let url = Url::parse(&format!("{CALLBACK_URL}?{raw}"))?;
    let pairs: Vec<_> = url.query_pairs().collect();
    let states: Vec<_> = pairs.iter().filter(|(key, _)| key == "state").collect();
    ensure!(
        states.len() == 1 && states[0].1 == expected_state,
        "Invalid callback state"
    );
    let codes: Vec<_> = pairs.iter().filter(|(key, _)| key == "code").collect();
    let errors = pairs.iter().filter(|(key, _)| key == "error").count();
    if errors > 0 {
        ensure!(errors == 1 && codes.is_empty(), "Invalid callback error");
        return Ok(Err(()));
    }
    ensure!(
        codes.len() == 1 && !codes[0].1.is_empty() && codes[0].1.len() <= 4096,
        "Login was not authorized"
    );
    Ok(Ok(Zeroizing::new(codes[0].1.to_string())))
}

async fn callback(
    State(flow): State<Arc<Flow>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> (StatusCode, &'static str) {
    // Invalid requests do not consume the outstanding flow.
    let Ok(result) = callback_code(&headers, raw.as_deref(), &flow.state) else {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid login callback. Return to Cirra to try again.",
        );
    };
    let Some(sender) = flow.result.lock().await.take() else {
        return (
            StatusCode::CONFLICT,
            "This login callback has already been used.",
        );
    };
    let _ = sender.send(result);
    flow.shutdown.notify_one();
    (StatusCode::OK, "Login received. You can return to Cirra.")
}

pub struct PendingLogin {
    pub authorize_url: String,
    config: LoginConfig,
    verifier: Zeroizing<String>,
    code: oneshot::Receiver<CallbackResult>,
    server: JoinHandle<()>,
    expires: Instant,
    consumed: bool,
}

pub async fn begin(config: LoginConfig) -> Result<PendingLogin> {
    let origin = domain(&config)?;
    // A previous flow's aborted task must get a scheduling opportunity to release
    // its listener before restarting. Never fall back to a different callback port.
    tokio::task::yield_now().await;
    let mut attempts = 0;
    let listener = loop {
        match tokio::net::TcpListener::bind("127.0.0.1:38317").await {
            Ok(listener) => break listener,
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse && attempts < 10 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(_) => bail!("The desktop login callback port is already in use"),
        }
    };
    let verifier = secret();
    let state = secret();
    let mut authorize = origin.join("/oauth2/authorize")?;
    authorize.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", CALLBACK_URL),
        ("scope", config.scope.as_str()),
        ("state", state.as_str()),
        ("code_challenge_method", "S256"),
        (
            "code_challenge",
            URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()))
                .as_str(),
        ),
    ]);
    let (sender, code) = oneshot::channel();
    let flow = Arc::new(Flow {
        state,
        result: Mutex::new(Some(sender)),
        shutdown: Notify::new(),
    });
    let router = Router::new()
        .route("/callback", get(callback))
        .with_state(flow.clone());
    let expires = Instant::now() + LIFETIME;
    let server = tokio::spawn(async move {
        let shutdown_flow = flow.clone();
        let serve = axum::serve(listener, router).with_graceful_shutdown(async move {
            shutdown_flow.shutdown.notified().await;
        });
        tokio::select! {
            _ = tokio::time::sleep_until(expires) => {},
            _ = async { serve.await } => {},
        }
    });
    Ok(PendingLogin {
        authorize_url: authorize.into(),
        config,
        verifier,
        code,
        server,
        expires,
        consumed: false,
    })
}

impl PendingLogin {
    pub async fn try_code(&mut self) -> Result<Option<OAuthTokens>> {
        ensure!(!self.consumed, "Login callback already consumed");
        if Instant::now() >= self.expires {
            self.server.abort();
            self.consumed = true;
            bail!("Login expired. Start a new login.");
        }
        let code = match self.code.try_recv() {
            Ok(Ok(code)) => code,
            Ok(Err(())) => {
                self.consumed = true;
                bail!("Login was not authorized");
            }
            Err(oneshot::error::TryRecvError::Empty) => return Ok(None),
            Err(oneshot::error::TryRecvError::Closed) => {
                self.consumed = true;
                bail!("Login callback listener stopped");
            }
        };
        self.consumed = true;
        self.server.abort();
        Ok(Some(
            exchange(&self.config, &self.verifier, &code, CALLBACK_URL).await?,
        ))
    }
}
impl Drop for PendingLogin {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// Exact callback registered by the iOS application and its account provider.
pub const IOS_CALLBACK_URL: &str = "app.cirra.ios:/oauth/callback";

/// Platform-authentication-session flow. The Rust owner keeps state and verifier;
/// only the authorization URL is passed to ASWebAuthenticationSession.
pub struct PlatformLogin {
    pub authorize_url: String,
    config: LoginConfig,
    verifier: Zeroizing<String>,
    state: Zeroizing<String>,
    expires: Instant,
    consumed: bool,
}
impl PlatformLogin {
    pub fn begin(config: LoginConfig) -> Result<Self> {
        let origin = domain(&config)?;
        let verifier = secret();
        let state = secret();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut authorize = origin.join("/oauth2/authorize")?;
        authorize.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", config.client_id.as_str()),
            ("redirect_uri", IOS_CALLBACK_URL),
            ("scope", config.scope.as_str()),
            ("state", state.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
        ]);
        Ok(Self {
            authorize_url: authorize.into(),
            config,
            verifier,
            state,
            expires: Instant::now() + LIFETIME,
            consumed: false,
        })
    }
    pub async fn complete(&mut self, callback: &str) -> Result<OAuthTokens> {
        ensure!(!self.consumed, "Login callback already consumed");
        self.consumed = true;
        ensure!(
            Instant::now() < self.expires,
            "Login expired. Start a new login."
        );
        ensure!(callback.len() <= 8192, "Callback is too large");
        let url = Url::parse(callback).map_err(|_| anyhow::anyhow!("Invalid login callback"))?;
        ensure!(
            url.scheme() == "app.cirra.ios"
                && url.host_str().is_none()
                && url.path() == "/oauth/callback"
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "Invalid login callback URL"
        );
        let pairs = url.query_pairs().collect::<Vec<_>>();
        let states = pairs
            .iter()
            .filter(|(key, _)| key == "state")
            .collect::<Vec<_>>();
        ensure!(
            states.len() == 1 && states[0].1 == self.state.as_str(),
            "Invalid callback state"
        );
        let codes = pairs
            .iter()
            .filter(|(key, _)| key == "code")
            .collect::<Vec<_>>();
        ensure!(
            !pairs.iter().any(|(key, _)| key == "error")
                && codes.len() == 1
                && !codes[0].1.is_empty()
                && codes[0].1.len() <= 4096,
            "Login was not authorized"
        );
        let code = Zeroizing::new(codes[0].1.to_string());
        exchange(&self.config, &self.verifier, &code, IOS_CALLBACK_URL).await
    }
}
async fn exchange(
    config: &LoginConfig,
    verifier: &str,
    code: &str,
    redirect_uri: &str,
) -> Result<OAuthTokens> {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    let mut response = http
        .post(domain(config)?.join("/oauth2/token")?)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", config.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("code", code),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Login token exchange failed"))?;
    ensure!(
        response.status().is_success(),
        "Login token exchange was rejected"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= 128 * 1024),
        "Login response is too large"
    );
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Unable to read login response"))?
    {
        ensure!(
            body.len() + chunk.len() <= 128 * 1024,
            "Login response is too large"
        );
        body.extend_from_slice(&chunk);
    }
    let tokens: OAuthTokens = serde_json::from_slice(&body)
        .map_err(|_| anyhow::anyhow!("Invalid login token response"))?;
    ensure!(
        !tokens.access_token.is_empty()
            && tokens.expires_in > 0
            && tokens.expires_in <= 86400
            && tokens.token_type.eq_ignore_ascii_case("bearer"),
        "Invalid login token response"
    );
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(domain: &str) -> LoginConfig {
        LoginConfig {
            issuer: "https://issuer.example/pool".into(),
            client_id: "client".into(),
            domain: domain.into(),
            scope: "openid cirra/access".into(),
        }
    }
    #[test]
    fn native_provider_url_is_confined_to_configured_origin() {
        let config = config("https://login.example");
        assert!(is_authorization_url(
            &config,
            "https://login.example/oauth2/authorize?client_id=client"
        ));
        for url in [
            "http://login.example/oauth2/authorize",
            "https://login.example.evil/oauth2/authorize",
            "https://login.example/oauth2/token",
            "https://user@login.example/oauth2/authorize",
        ] {
            assert!(!is_authorization_url(&config, url));
        }
        assert!(domain(&self::config("http://login.example")).is_err());
        assert!(domain(&self::config("http://127.0.0.1:41111")).is_ok());
    }
    #[test]
    fn callback_requires_exact_host_and_one_matching_state_and_code() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "localhost:38317".parse().unwrap());
        assert_eq!(
            callback_code(&headers, Some("state=expected&code=valid"), "expected")
                .unwrap()
                .unwrap()
                .as_str(),
            "valid"
        );
        for query in [
            "state=wrong&code=valid",
            "state=expected&state=expected&code=valid",
            "state=expected&code=one&code=two",
            "code=valid",
        ] {
            assert!(callback_code(&headers, Some(query), "expected").is_err());
        }
        assert!(
            callback_code(
                &headers,
                Some("state=expected&error=access_denied"),
                "expected"
            )
            .unwrap()
            .is_err()
        );
        headers.insert("host", "evil.example:38317".parse().unwrap());
        assert!(callback_code(&headers, Some("state=expected&code=valid"), "expected").is_err());
    }
    #[tokio::test]
    async fn pkce_exchange_stays_native_and_callback_is_one_use() {
        use axum::{Form, Json};
        use std::collections::BTreeMap;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let challenge = Arc::new(Mutex::new(String::new()));
        let expected = challenge.clone();
        let app = Router::new().route("/oauth2/token", axum::routing::post(move |Form(form): Form<BTreeMap<String,String>>| {
            let expected = expected.clone();
            async move {
                assert_eq!(form.get("code").unwrap(), "native-code");
                assert_eq!(form.get("redirect_uri").unwrap(), CALLBACK_URL);
                assert_eq!(form.get("client_id").unwrap(), "client");
                assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(form.get("code_verifier").unwrap().as_bytes())), *expected.lock().await);
                Json(serde_json::json!({"access_token":"native-access","refresh_token":"native-refresh","token_type":"Bearer","expires_in":3600}))
            }
        }));
        let fixture = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut pending = begin(config(&format!("http://{address}"))).await.unwrap();
        let authorize = Url::parse(&pending.authorize_url).unwrap();
        let parameters: BTreeMap<_, _> = authorize
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        *challenge.lock().await = parameters.get("code_challenge").unwrap().clone();
        assert_eq!(parameters.get("code_challenge_method").unwrap(), "S256");
        assert!(!parameters.contains_key("code_verifier"));
        assert!(pending.try_code().await.unwrap().is_none());
        let http = reqwest::Client::new();
        let callback = format!(
            "http://127.0.0.1:38317/callback?state={}&code=native-code",
            parameters.get("state").unwrap()
        );
        assert_eq!(
            http.get(&callback).send().await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
        assert!(pending.try_code().await.unwrap().is_none());
        assert!(
            http.get(&callback)
                .header("Host", "localhost:38317")
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        );
        let tokens = pending.try_code().await.unwrap().unwrap();
        assert_eq!(tokens.access_token.as_str(), "native-access");
        assert_eq!(tokens.refresh_token.unwrap().as_str(), "native-refresh");
        assert!(pending.try_code().await.is_err());
        drop(pending);
        fixture.abort();
    }
    #[tokio::test]
    async fn ios_callback_requires_exact_url_state_and_single_use() {
        for callback in [
            "https://login.example/oauth/callback?state=bad&code=one",
            "app.cirra.ios://other/oauth/callback?state=bad&code=one",
            "app.cirra.ios:/wrong?state=bad&code=one",
            "app.cirra.ios:/oauth/callback?state=bad&code=one",
        ] {
            let mut flow = PlatformLogin::begin(config("https://login.example")).unwrap();
            assert!(flow.complete(callback).await.is_err());
            assert!(flow.complete(callback).await.is_err());
        }
        let mut expired = PlatformLogin::begin(config("https://login.example")).unwrap();
        expired.expires = Instant::now();
        assert!(
            expired
                .complete("app.cirra.ios:/oauth/callback")
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn ios_pkce_exchange_keeps_verifier_and_tokens_in_rust() {
        use axum::{Form, Json};
        use std::collections::BTreeMap;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected = Arc::new(Mutex::new(String::new()));
        let check = expected.clone();
        let app=Router::new().route("/oauth2/token",axum::routing::post(move|Form(form):Form<BTreeMap<String,String>>|{let check=check.clone();async move{
            assert_eq!(form["redirect_uri"],IOS_CALLBACK_URL);assert_eq!(form["code"],"ios-code");
            assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),*check.lock().await);
            Json(serde_json::json!({"access_token":"ios-access","token_type":"Bearer","expires_in":300}))
        }}));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut flow = PlatformLogin::begin(config(&format!("http://{address}"))).unwrap();
        let url = Url::parse(&flow.authorize_url).unwrap();
        let parameters = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(parameters["redirect_uri"], IOS_CALLBACK_URL);
        assert_eq!(parameters["code_challenge_method"], "S256");
        assert!(!parameters.contains_key("code_verifier"));
        *expected.lock().await = parameters["code_challenge"].clone();
        let callback = format!(
            "{IOS_CALLBACK_URL}?state={}&code=ios-code",
            parameters["state"]
        );
        assert_eq!(
            flow.complete(&callback)
                .await
                .unwrap()
                .access_token
                .as_str(),
            "ios-access"
        );
        assert!(flow.complete(&callback).await.is_err());
        server.abort();
    }
}
