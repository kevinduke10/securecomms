//! Loopback browser sessions. Demo identity selection is NEVER a cloud login provider.
use super::*;
use axum::{
    extract::Request,
    http::{HeaderMap, header},
    response::Redirect,
};
use croaker_relay::{
    identity::{AccountGrant, identity_id},
    login::{LoginConfig, LoginVerifier},
};
use sha2::{Digest, Sha256};

pub struct AuthOptions {
    pub auto_login: bool,
    pub config: Option<LoginConfig>,
    pub base_url: Option<String>,
    pub subjects: HashMap<String, String>,
}
impl AuthOptions {
    pub fn demo(auto_login: bool) -> Self {
        Self {
            auto_login,
            config: None,
            base_url: None,
            subjects: HashMap::new(),
        }
    }
    pub fn from_env(port: u16) -> Result<Self> {
        match std::env::var("CROAKER_LOGIN_MODE")
            .as_deref()
            .unwrap_or("demo")
        {
            "demo" => Ok(Self::demo(
                std::env::var("CROAKER_DEMO_AUTOLOGIN").as_deref() == Ok("1"),
            )),
            "cognito" => {
                ensure!(
                    std::env::var("CROAKER_DEMO_AUTOLOGIN").as_deref() != Ok("1"),
                    "Automatic demo login is forbidden in Cognito mode."
                );
                let subjects = serde_json::from_slice(&std::fs::read(std::env::var(
                    "CROAKER_COGNITO_SUBJECTS_FILE",
                )?)?)?;
                let base = std::env::var("CROAKER_LOGIN_BASE_URL")
                    .unwrap_or_else(|_| format!("http://localhost:{port}"));
                ensure!(
                    base == format!("http://localhost:{port}")
                        || base == format!("http://127.0.0.1:{port}"),
                    "Local login base URL must match this loopback listener."
                );
                Ok(Self {
                    auto_login: false,
                    config: Some(LoginConfig::from_env()?),
                    base_url: Some(base),
                    subjects,
                })
            }
            _ => anyhow::bail!("Use CROAKER_LOGIN_MODE=demo or cognito."),
        }
    }
    fn issuer(&self) -> &str {
        self.config
            .as_ref()
            .map(|c| c.issuer.as_str())
            .unwrap_or("https://croaker.local.test")
    }
    fn subject(&self, owner: &str) -> Result<String> {
        if self.config.is_some() {
            self.subjects
                .get(owner)
                .cloned()
                .context("Every local demo owner needs an explicitly configured Cognito subject.")
        } else {
            Ok(owner.into())
        }
    }
}
impl Workspace {
    pub async fn configure_login(&mut self, options: &AuthOptions) -> Result<()> {
        for owner in ["alice", "bob", "maya"] {
            let profile = self.profile(owner)?;
            let grant = AccountGrant {
                issuer: options.issuer().into(),
                subject: options.subject(owner)?,
                tenant_id: profile.principal.tenant_id,
                role: profile.principal.role,
                name: profile.name,
                bootstrap_device: Some(croaker_protocol::enrollment::IdentityBinding {
                    tenant_id: profile.binding.tenant_id,
                    device_id: profile.binding.device_id,
                    signature_key: profile.binding.signature_key,
                }),
            };
            self.relay.grant_account(grant).await?;
        }
        for profile in &mut self.manifest.profiles {
            if profile.owner.is_none() {
                profile.owner = Some(profile.alias.clone());
            }
        }
        self.save()?;
        Ok(())
    }
}
#[derive(Clone)]
pub(super) struct Session {
    pub account_id: String,
    pub owner: String,
    pub active_alias: String,
    pub expires: u64,
}
struct Flow {
    verifier: Zeroizing<String>,
    cookie_hash: String,
    expires: u64,
}
struct Memory {
    sessions: HashMap<String, Session>,
    flows: HashMap<String, Flow>,
}
pub(super) struct AuthService {
    pub workspace: Shared,
    pub options: AuthOptions,
    verifier: Option<LoginVerifier>,
    memory: Mutex<Memory>,
    port: u16,
}
fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    let mut values = raw
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(key, _)| *key == name);
    let value = values.next()?.1;
    if values.next().is_some() || value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit())
    {
        None
    } else {
        Some(value.into())
    }
}
fn session_cookie(value: &str, seconds: u64) -> String {
    format!("croaker_session={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={seconds}")
}
impl AuthService {
    pub(super) fn new(workspace: Shared, options: AuthOptions, port: u16) -> Arc<Self> {
        assert!(
            !options.auto_login || options.config.is_none(),
            "Cognito mode cannot bypass login"
        );
        let verifier = options
            .config
            .clone()
            .map(|config| LoginVerifier::new(config).expect("validated login configuration"));
        Arc::new(Self {
            workspace,
            options,
            verifier,
            memory: Mutex::new(Memory {
                sessions: HashMap::new(),
                flows: HashMap::new(),
            }),
            port,
        })
    }
    pub(super) async fn session(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<Session, StatusCode> {
        let token = cookie(headers, "croaker_session").ok_or(StatusCode::UNAUTHORIZED)?;
        let mut memory = self.memory.lock().await;
        memory
            .sessions
            .retain(|_, s| s.expires > croaker_relay::now());
        memory
            .sessions
            .get(&digest(&token))
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
    pub(super) async fn set_active(&self, headers: &HeaderMap, alias: String) -> Result<()> {
        let token = cookie(headers, "croaker_session").context("Sign in again.")?;
        let mut memory = self.memory.lock().await;
        let session = memory
            .sessions
            .get_mut(&digest(&token))
            .context("Sign in again.")?;
        session.active_alias = alias;
        Ok(())
    }
    async fn issue(&self, owner: String, expires: u64) -> Result<(Session, String)> {
        let account_id = identity_id(self.options.issuer(), &self.options.subject(&owner)?);
        let active_alias = {
            let workspace = self.workspace.lock().await;
            workspace.relay.account(&account_id).await?;
            let mut selected = owner.clone();
            for profile in workspace
                .manifest
                .profiles
                .iter()
                .filter(|p| p.owner.as_deref().unwrap_or(&p.alias) == owner)
            {
                if workspace.relay.authenticate(&profile.token).await.is_ok() {
                    selected = profile.alias.clone();
                    break;
                }
            }
            selected
        };
        let token = hex::encode(*random_key());
        let session = Session {
            account_id,
            active_alias,
            owner,
            expires,
        };
        let mut memory = self.memory.lock().await;
        memory
            .sessions
            .retain(|_, s| s.expires > croaker_relay::now());
        ensure!(memory.sessions.len() < 1000, "Local session limit reached.");
        memory.sessions.insert(digest(&token), session.clone());
        Ok((session, token))
    }
    pub(super) async fn guard(&self, request: Request, next: Next) -> Response {
        if self.options.auto_login {
            return next.run(request).await;
        }
        let session = match self.session(request.headers()).await {
            Ok(s) => s,
            Err(status) => {
                return (
                    status,
                    Json(serde_json::json!({"error":"Sign in to your account."})),
                )
                    .into_response();
            }
        };
        let path = request.uri().path();
        let alias = if path == "/state" {
            request
                .uri()
                .query()
                .and_then(|query| reqwest::Url::parse(&format!("http://localhost/?{query}")).ok())
                .and_then(|url| {
                    url.query_pairs()
                        .find(|(key, _)| key == "profile")
                        .map(|(_, value)| value.into_owned())
                })
        } else {
            path.trim_start_matches('/')
                .split('/')
                .next()
                .map(str::to_string)
        };
        let permitted = {
            let workspace = self.workspace.lock().await;
            alias
                .as_ref()
                .and_then(|alias| {
                    workspace
                        .manifest
                        .profiles
                        .iter()
                        .find(|p| p.alias == *alias)
                })
                .is_some_and(|p| p.owner.as_deref().unwrap_or(&p.alias) == session.owner)
        };
        if !permitted {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error":"This device belongs to another account."})),
            )
                .into_response();
        }
        next.run(request).await
    }
    fn same_origin(&self, headers: &HeaderMap) -> bool {
        let hosts = [
            format!("127.0.0.1:{}", self.port),
            format!("localhost:{}", self.port),
            format!("[::1]:{}", self.port),
        ];
        headers
            .get("host")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| hosts.iter().any(|host| host == h))
            && headers.get("origin").is_none_or(|v| {
                v.to_str()
                    .ok()
                    .is_some_and(|v| hosts.iter().any(|h| v == format!("http://{h}")))
            })
            && headers
                .get("sec-fetch-site")
                .is_none_or(|h| matches!(h.to_str(), Ok("same-origin" | "none")))
    }
}
async fn info(
    State(auth): State<Arc<AuthService>>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, LocalError> {
    if auth.options.auto_login {
        return Ok(Json(
            serde_json::json!({"authenticated":true,"auto_login":true,"mode":"demo"}),
        ));
    }
    let session = auth.session(&headers).await.ok();
    let mut info = serde_json::json!({"authenticated":session.is_some(),"auto_login":false,"mode":if auth.options.config.is_some(){"cognito"}else{"demo"}});
    if let Some(session) = session {
        let workspace = auth.workspace.lock().await;
        let account = workspace.relay.account(&session.account_id).await?;
        let aliases:Vec<_>=workspace.manifest.profiles.iter().filter(|p|p.owner.as_deref().unwrap_or(&p.alias)==session.owner).map(|p|serde_json::json!({"alias":p.alias,"device_id":p.principal.device_id,"label":p.title})).collect();
        info["profile"] = session.active_alias.into();
        info["owner"] = session.owner.into();
        info["account"] = account.public();
        info["local_devices"] = serde_json::json!(aliases);
    }
    Ok(Json(info))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DemoLogin {
    profile: String,
}
async fn demo_login(
    State(auth): State<Arc<AuthService>>,
    Json(input): Json<DemoLogin>,
) -> std::result::Result<Response, LocalError> {
    if auth.options.config.is_some() || !["alice", "bob", "maya"].contains(&input.profile.as_str())
    {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }
    let (_, token) = auth
        .issue(input.profile, croaker_relay::now() + 8 * 3600)
        .await?;
    Ok((
        [(header::SET_COOKIE, session_cookie(&token, 8 * 3600))],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}
async fn logout(
    State(auth): State<Arc<AuthService>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    if let Some(token) = cookie(&headers, "croaker_session") {
        auth.memory.lock().await.sessions.remove(&digest(&token));
    }
    let redirect = auth.options.config.as_ref().map(|config| {
        let mut url = reqwest::Url::parse(&format!("{}/logout", config.domain)).unwrap();
        url.query_pairs_mut()
            .append_pair("client_id", &config.client_id)
            .append_pair("logout_uri", auth.options.base_url.as_deref().unwrap());
        url.to_string()
    });
    Json(serde_json::json!({"redirect":redirect}))
}
async fn start(
    State(auth): State<Arc<AuthService>>,
    headers: HeaderMap,
) -> std::result::Result<Response, LocalError> {
    let config = auth
        .options
        .config
        .as_ref()
        .context("Cognito login is not configured.")?;
    let base = auth.options.base_url.as_ref().unwrap();
    let host = reqwest::Url::parse(base)
        .map_err(anyhow::Error::from)?
        .authority()
        .to_string();
    if headers.get("host").and_then(|h| h.to_str().ok()) != Some(host.as_str()) {
        return Ok(Redirect::to(&format!("{base}/local-auth/start")).into_response());
    }
    let state = hex::encode(*random_key());
    let browser = hex::encode(*random_key());
    let verifier = Zeroizing::new(hex::encode(*random_key()));
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    {
        let mut memory = auth.memory.lock().await;
        memory.flows.retain(|_, f| f.expires > croaker_relay::now());
        if memory.flows.len() >= 100 {
            return Err(anyhow::anyhow!("Too many login attempts.").into());
        }
        memory.flows.insert(
            digest(&state),
            Flow {
                verifier: verifier.clone(),
                cookie_hash: digest(&browser),
                expires: croaker_relay::now() + 300,
            },
        );
    }
    let mut url = reqwest::Url::parse(&format!("{}/oauth2/authorize", config.domain))
        .map_err(anyhow::Error::from)?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &format!("{base}/local-auth/callback"))
        .append_pair("scope", &config.scope)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    Ok((
        [(
            header::SET_COOKIE,
            format!(
                "croaker_login={browser}; Path=/local-auth; HttpOnly; SameSite=Lax; Max-Age=300"
            ),
        )],
        Redirect::to(url.as_str()),
    )
        .into_response())
}
#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}
async fn callback(
    State(auth): State<Arc<AuthService>>,
    headers: HeaderMap,
    Query(input): Query<Callback>,
) -> std::result::Result<Response, LocalError> {
    // Never echo authorization codes, tokens, or provider error text.
    let result: Result<String> = async {
        let config = auth.options.config.as_ref().context("Login unavailable")?;
        ensure!(input.error.is_none(), "Login declined");
        let state = input
            .state
            .as_ref()
            .filter(|state| state.len() == 64)
            .context("Login state missing")?;
        let browser = cookie(&headers, "croaker_login").context("Login cookie missing")?;
        let flow = auth
            .memory
            .lock()
            .await
            .flows
            .remove(&digest(state))
            .context("Login state expired")?;
        ensure!(
            flow.expires > croaker_relay::now() && flow.cookie_hash == digest(&browser),
            "Login state mismatch"
        );
        let code = input
            .code
            .as_ref()
            .filter(|c| c.len() <= 4096)
            .context("Login code missing")?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let response = http
            .post(format!("{}/oauth2/token", config.domain))
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", config.client_id.as_str()),
                ("code", code),
                ("code_verifier", flow.verifier.as_str()),
                (
                    "redirect_uri",
                    &format!(
                        "{}/local-auth/callback",
                        auth.options.base_url.as_ref().unwrap()
                    ),
                ),
            ])
            .send()
            .await?;
        ensure!(response.status().is_success(), "Token exchange failed");
        let mut response = response;
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 64 * 1024,
                "Token response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        #[derive(Deserialize)]
        struct Tokens {
            access_token: String,
        }
        let tokens: Tokens = serde_json::from_slice(&bytes)?;
        let access = Zeroizing::new(tokens.access_token);
        let claims = auth.verifier.as_ref().unwrap().verify(&access).await?;
        let owner = auth
            .options
            .subjects
            .iter()
            .find(|(_, subject)| **subject == claims.sub)
            .map(|(alias, _)| alias.clone())
            .context("Account not provisioned")?;
        let (_, token) = auth.issue(owner, claims.exp).await?;
        Ok(token)
    }
    .await;
    match result {
        Ok(token) => Ok((
            [(header::SET_COOKIE, session_cookie(&token, 300))],
            Redirect::to("/"),
        )
            .into_response()),
        Err(_) => Ok(Redirect::to("/?login_error=1").into_response()),
    }
}
pub(super) fn routes(auth: Arc<AuthService>) -> Router {
    let guard = auth.clone();
    Router::new()
        .route("/local-auth/me", get(info))
        .route("/local-auth/demo", post(demo_login))
        .route(
            "/local-auth/logout",
            post(logout).layer(middleware::map_response(
                |mut response: Response| async move {
                    response.headers_mut().insert(
                        header::SET_COOKIE,
                        HeaderValue::from_static(
                            "croaker_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
                        ),
                    );
                    response
                },
            )),
        )
        .route("/local-auth/start", get(start))
        .route("/local-auth/callback", get(callback))
        .merge(crate::enrollment::routes())
        .layer(DefaultBodyLimit::max(8 * 1024))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let auth = guard.clone();
            async move {
                let callback = request.uri().path() == "/local-auth/callback";
                if !callback
                    && (!auth.same_origin(request.headers())
                        || (request.method() != axum::http::Method::GET
                            && request
                                .headers()
                                .get("x-croaker-local")
                                .and_then(|v| v.to_str().ok())
                                != Some("1")))
                {
                    return StatusCode::FORBIDDEN.into_response();
                }
                // Callback is cross-site but must still reach the configured Host.
                if callback {
                    let expected = auth
                        .options
                        .base_url
                        .as_ref()
                        .and_then(|v| reqwest::Url::parse(v).ok())
                        .map(|v| v.authority().to_string());
                    if request.headers().get("host").and_then(|v| v.to_str().ok())
                        != expected.as_deref()
                    {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                }
                next.run(request).await
            }
        }))
        .with_state(auth)
}
