//! Loopback-only disposable browser contract fixture. Never part of Lambda.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use croaker_protocol::*;
use croaker_relay::{Store, identity::AccountGrant, sqlite::SqliteStore};
use serde_json::{Value, json};
use std::sync::Arc;
#[derive(Clone)]
struct Fixture {
    store: Arc<SqliteStore>,
    accounts: Vec<String>,
    operator: Principal,
}
type Answer<T> = Result<T, (StatusCode, &'static str)>;
fn error(_: impl std::fmt::Display) -> (StatusCode, &'static str) {
    (StatusCode::BAD_REQUEST, "Fixture request rejected")
}
fn account(headers: &HeaderMap, state: &Fixture) -> Answer<String> {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let i = match token {
        "Bearer fixture-alice" => 0,
        "Bearer fixture-bob" => 1,
        _ => return Err((StatusCode::UNAUTHORIZED, "Sign in")),
    };
    Ok(state.accounts[i].clone())
}
async fn me(State(state): State<Fixture>, headers: HeaderMap) -> Answer<Json<Value>> {
    let id = account(&headers, &state)?;
    Ok(Json(
        state.store.account(&id).await.map_err(error)?.public(),
    ))
}
async fn enroll(
    State(state): State<Fixture>,
    headers: HeaderMap,
    Json(input): Json<enrollment::EnrollmentInput>,
) -> Answer<StatusCode> {
    let id = account(&headers, &state)?;
    state
        .store
        .request_enrollment(&id, input)
        .await
        .map_err(error)?;
    Ok(StatusCode::ACCEPTED)
}
async fn inspect(
    State(state): State<Fixture>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Answer<Json<Value>> {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    let actor = state.store.authenticate(token).await.map_err(error)?;
    let record = state.store.account(&id).await.map_err(error)?;
    record.can_approve(&actor).map_err(error)?;
    Ok(Json(record.public()))
}
async fn approve(State(state): State<Fixture>, Json(body): Json<Value>) -> Answer<StatusCode> {
    let id = body["account"]
        .as_str()
        .ok_or((StatusCode::BAD_REQUEST, "account"))?;
    let record = state.store.account(id).await.map_err(error)?;
    let request = record
        .requests
        .last()
        .ok_or((StatusCode::BAD_REQUEST, "request"))?;
    let fp = body["fingerprint"]
        .as_str()
        .ok_or((StatusCode::BAD_REQUEST, "fingerprint"))?;
    state
        .store
        .decide_enrollment(&state.operator, id, request.input.id, fp, true)
        .await
        .map_err(error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = Arc::new(SqliteStore::open(":memory:")?);
    let tenant = TenantId::new();
    let operator = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Admin,
    };
    store.provision(&operator, &"ab".repeat(32)).await?;
    let mut accounts = vec![];
    for name in ["Alice", "Bob"] {
        let grant = AccountGrant {
            issuer: "https://fixture.example/pool".into(),
            subject: name.into(),
            tenant_id: tenant,
            role: Role::Member,
            name: name.into(),
            bootstrap_device: None,
        };
        accounts.push(grant.id());
        store.grant_account(grant).await?;
    }
    let state = Fixture {
        store: store.clone(),
        accounts,
        operator,
    };
    let local=Router::new().route("/v1/account",get(me)).route("/v1/account/enrollments",post(enroll)).route("/v1/enrollments/{id}",get(inspect)).route("/test/approve",post(approve)).route("/v1/login/config",get(||async{Json(json!({"issuer":"https://fixture.example/pool","client_id":"fixture","domain":"https://login.fixture.example","scope":"openid croaker/access"}))})).with_state(state);
    let files = tempfile::tempdir()?;
    let media = Arc::new(croaker_relay::media::ObjectService {
        relay: store.clone(),
        registry: Arc::new(croaker_relay::media::local::SqliteRegistry::open(
            files.path().join("media.sqlite"),
        )?),
        objects: Arc::new(croaker_relay::media::local::Files::open(
            files.path().join("objects"),
            "http://127.0.0.1:3111".into(),
        )?),
    });
    let router = croaker_relay::api::router(store)
        .merge(local)
        .merge(croaker_relay::media::router(media.clone()))
        .merge(croaker_relay::media::local_content_router(media));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3112").await?;
    println!("Disposable browser fixture: http://127.0.0.1:3112");
    axum::serve(listener, router).await?;
    Ok(())
}
