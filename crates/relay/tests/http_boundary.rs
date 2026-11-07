use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_protocol::*;
use croaker_relay::{Store, api, sqlite::SqliteStore};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn authenticated_http_enforces_scope_schema_and_body_limits() {
    let store = Arc::new(SqliteStore::open(":memory:").unwrap());
    let tenant = TenantId::new();
    let alice = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Admin,
    };
    let bob = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Member,
    };
    let outsider = Principal {
        tenant_id: TenantId::new(),
        device_id: DeviceId::new(),
        role: Role::Admin,
    };
    for (principal, token) in [(&alice, "a"), (&bob, "b"), (&outsider, "c")] {
        store.provision(principal, &token.repeat(64)).await.unwrap();
    }
    let app = api::router(store);
    let id = ConversationId::new();
    let create = serde_json::to_vec(&CreateConversation {
        id,
        devices: vec![alice.device_id, bob.device_id],
    })
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/conversations")
                .header("content-type", "application/json")
                .body(Body::from(create.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/conversations")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", "a".repeat(64)))
                .body(Body::from(create))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let event = SubmitEvent {
        protocol_version: API_VERSION,
        message_id: MessageId::new(),
        kind: EventKind::Application,
        recipient: None,
        ciphertext: BASE64_STANDARD.encode(b"opaque MLS message"),
    };
    let bytes = serde_json::to_vec(&event).unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/v1/conversations/{id}/events"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", "a".repeat(64)))
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/v1/conversations/{id}/sync"))
                .header("authorization", format!("Bearer {}", "c".repeat(64)))
                .header("x-tenant-id", tenant.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error: ApiError =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
    assert_eq!(error.code, "not_found");
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/v1/conversations/{id}/sync"))
                .header("authorization", format!("Bearer {}", "b".repeat(64)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: SyncPage =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(page.events.len(), 1);
    let mut injected = serde_json::to_value(event).unwrap();
    injected["tenant_id"] = serde_json::json!(tenant);
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/v1/conversations/{id}/events"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", "a".repeat(64)))
                .body(Body::from(serde_json::to_vec(&injected).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = app
        .oneshot(
            Request::post(format!("/v1/conversations/{id}/events"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", "a".repeat(64)))
                .body(Body::from(vec![b' '; 100 * 1024]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
