use crate::{Store, StoreError};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use croaker_protocol::*;
use serde::Deserialize;
use std::sync::Arc;

type Backend = Arc<dyn Store>;

pub fn router(store: Backend) -> Router {
    Router::new()
        .route(
            "/healthz",
            get(|| async {
                Json(serde_json::json!({"status":"ok", "protocol_version": API_VERSION}))
            }),
        )
        .route("/v1/conversations", post(create))
        .route("/v1/conversations/{id}", get(conversation_info))
        .route(
            "/v1/conversations/{id}/admissions",
            post(admit).layer(DefaultBodyLimit::max(128 * 1024)),
        )
        .route("/v1/conversations/{id}/events", post(append))
        .route("/v1/conversations/{id}/sync", get(sync))
        .route("/v1/devices/{id}/revoke", post(revoke))
        .layer(DefaultBodyLimit::max(96 * 1024))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert("cache-control", "no-store".parse().unwrap());
                response
                    .headers_mut()
                    .insert("x-content-type-options", "nosniff".parse().unwrap());
                response
            },
        ))
        .with_state(store)
}

async fn authenticate(headers: &HeaderMap, store: &Backend) -> Result<Principal, StoreError> {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(StoreError::Unauthorized)?;
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(StoreError::Unauthorized);
    }
    store.authenticate(token).await
}

async fn create(
    State(store): State<Backend>,
    headers: HeaderMap,
    Json(input): Json<CreateConversation>,
) -> Result<(StatusCode, Json<Conversation>), StoreError> {
    let actor = authenticate(&headers, &store).await?;
    Ok((
        StatusCode::CREATED,
        Json(store.create(&actor, input).await?),
    ))
}

async fn conversation_info(
    State(store): State<Backend>,
    headers: HeaderMap,
    Path(id): Path<ConversationId>,
) -> Result<Json<Conversation>, StoreError> {
    let actor = authenticate(&headers, &store).await?;
    Ok(Json(store.conversation_info(&actor, id).await?))
}
async fn admit(
    State(store): State<Backend>,
    headers: HeaderMap,
    Path(id): Path<ConversationId>,
    Json(input): Json<membership::GroupAdmission>,
) -> Result<Json<membership::AcceptedAdmission>, StoreError> {
    let actor = authenticate(&headers, &store).await?;
    Ok(Json(store.admit(&actor, id, input).await?))
}

async fn append(
    State(store): State<Backend>,
    headers: HeaderMap,
    Path(id): Path<ConversationId>,
    Json(input): Json<SubmitEvent>,
) -> Result<Json<AcceptedEvent>, StoreError> {
    let actor = authenticate(&headers, &store).await?;
    Ok(Json(store.append(&actor, id, input).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncQuery {
    #[serde(default)]
    after: u64,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    MAX_SYNC_PAGE
}

async fn sync(
    State(store): State<Backend>,
    headers: HeaderMap,
    Path(id): Path<ConversationId>,
    Query(query): Query<SyncQuery>,
) -> Result<Json<SyncPage>, StoreError> {
    let actor = authenticate(&headers, &store).await?;
    Ok(Json(
        store.sync(&actor, id, query.after, query.limit).await?,
    ))
}

async fn revoke(
    State(store): State<Backend>,
    headers: HeaderMap,
    Path(id): Path<DeviceId>,
) -> Result<StatusCode, StoreError> {
    let actor = authenticate(&headers, &store).await?;
    store.revoke(&actor, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

impl IntoResponse for StoreError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict => (StatusCode::CONFLICT, "conflict"),
            Self::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_request"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        };
        if matches!(self, Self::Unavailable) {
            tracing::error!("storage operation failed");
        }
        let mut response = (
            status,
            Json(ApiError {
                code: code.into(),
                message: self.to_string(),
            }),
        )
            .into_response();
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().unwrap());
        response
    }
}

/// Account endpoints use Cognito JWTs; messaging endpoints retain approved-device credentials.
pub fn router_with_login(store: Backend, verifier: Arc<crate::login::LoginVerifier>) -> Router {
    router(store.clone())
        .merge(crate::login::routes(store, verifier))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert("cache-control", "no-store".parse().unwrap());
                response
                    .headers_mut()
                    .insert("x-content-type-options", "nosniff".parse().unwrap());
                response
            },
        ))
}
