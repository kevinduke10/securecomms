use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use croaker_local::{Content, Workspace, demo_test_router};
use croaker_protocol::*;
use serde_json::{Value, json};
use tower::ServiceExt;
fn conversation(state: &Value, index: usize) -> ConversationId {
    serde_json::from_value(state["conversations"][index]["id"].clone()).unwrap()
}
fn content(text: &str) -> Content {
    Content {
        version: 1,
        text: text.into(),
        attachment: None,
    }
}
fn find(state: &Value, chat: ConversationId, id: MessageId) -> &Value {
    state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == chat.to_string())
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id.to_string())
        .unwrap()
}
#[tokio::test]
async fn replies_resolve_authenticated_targets_and_survive_restart_without_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(dir.path().into()).await.unwrap();
    let initial = workspace.snapshot("alice").await.unwrap();
    let chat = conversation(&initial, 0);
    let other = conversation(&initial, 1);
    let target = MessageId::new();
    workspace
        .send(
            "alice",
            chat,
            target,
            content("Authenticated original 619573"),
        )
        .await
        .unwrap();
    let reply = MessageId::new();
    for _ in 0..2 {
        workspace
            .send_reply(
                "bob",
                chat,
                reply,
                content("Authenticated reply 453819"),
                target,
                "alice",
            )
            .await
            .unwrap();
    }
    let state = workspace.snapshot("alice").await.unwrap();
    let message = find(&state, chat, reply);
    assert_eq!(message["reply_to"]["text"], "Authenticated original 619573");
    assert_eq!(message["reply_to"]["sender"], "alice");
    assert_eq!(message["reply_to"]["available"], true);
    assert!(
        workspace
            .send_reply(
                "bob",
                other,
                MessageId::new(),
                content("Not a valid cross-conversation quote"),
                target,
                "alice"
            )
            .await
            .is_err()
    );
    assert!(
        workspace
            .send_reply(
                "bob",
                chat,
                MessageId::new(),
                content("Wrong target sender"),
                target,
                "maya"
            )
            .await
            .is_err()
    );
    drop(workspace);
    let mut workspace = Workspace::open(dir.path().into()).await.unwrap();
    let state = workspace.snapshot("bob").await.unwrap();
    assert_eq!(
        find(&state, chat, reply)["reply_to"]["text"],
        "Authenticated original 619573"
    );
    for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
        let bytes = std::fs::read(entry.path()).unwrap();
        for marker in [
            b"Authenticated original 619573".as_slice(),
            b"Authenticated reply 453819".as_slice(),
        ] {
            assert!(!bytes.windows(marker.len()).any(|w| w == marker));
        }
    }
}
#[tokio::test]
async fn delivery_receipts_follow_receive_and_read_is_opt_in_and_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(dir.path().into()).await.unwrap();
    let initial = workspace.snapshot("alice").await.unwrap();
    let chat = conversation(&initial, 0);
    let target = MessageId::new();
    workspace
        .send("alice", chat, target, content("Receipt test body"))
        .await
        .unwrap();
    let before = workspace.snapshot("alice").await.unwrap();
    assert!(
        find(&before, chat, target)["receipts"]["delivered"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let bob = workspace.snapshot("bob").await.unwrap();
    assert_eq!(bob["read_receipts"], false);
    let count = bob["conversations"][0]["messages"]
        .as_array()
        .unwrap()
        .len();
    assert!(
        workspace
            .mark_read("bob", chat, vec![(target, "alice".into())])
            .await
            .is_err()
    );
    let delivered = workspace.snapshot("alice").await.unwrap();
    assert_eq!(
        find(&delivered, chat, target)["receipts"]["delivered"],
        json!(["bob"])
    );
    assert_eq!(
        find(&delivered, chat, target)["receipts"]["read"],
        json!([])
    );
    workspace.set_read_receipts("bob", true).unwrap();
    for _ in 0..3 {
        workspace
            .mark_read("bob", chat, vec![(target, "alice".into())])
            .await
            .unwrap();
    }
    let read = workspace.snapshot("alice").await.unwrap();
    assert_eq!(
        find(&read, chat, target)["receipts"]["read"],
        json!(["bob"])
    );
    assert_eq!(
        find(&read, chat, target)["receipts"]["delivered"],
        json!(["bob"])
    );
    assert!(
        workspace
            .mark_read("bob", chat, vec![(target, "maya".into())])
            .await
            .is_err()
    );
    assert!(
        workspace
            .mark_read("alice", chat, vec![(target, "alice".into())])
            .await
            .is_err()
    );
    for _ in 0..3 {
        assert_eq!(
            workspace.snapshot("bob").await.unwrap()["conversations"][0]["messages"]
                .as_array()
                .unwrap()
                .len(),
            count
        );
        workspace.snapshot("alice").await.unwrap();
    }
    drop(workspace);
    let mut workspace = Workspace::open(dir.path().into()).await.unwrap();
    assert_eq!(
        workspace.snapshot("bob").await.unwrap()["read_receipts"],
        true
    );
    assert_eq!(
        find(&workspace.snapshot("alice").await.unwrap(), chat, target)["receipts"]["read"],
        json!(["bob"])
    );
}
#[tokio::test]
async fn offline_read_receipt_retries_saved_event_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(dir.path().into()).await.unwrap();
    let initial = workspace.snapshot("alice").await.unwrap();
    let chat = conversation(&initial, 0);
    let target = MessageId::new();
    workspace
        .send("alice", chat, target, content("Offline read fixture"))
        .await
        .unwrap();
    workspace.snapshot("bob").await.unwrap();
    workspace.set_read_receipts("bob", true).unwrap();
    let app = demo_test_router(workspace, std::path::PathBuf::from("missing"), 3000);
    async fn post(app: &axum::Router, path: &str, body: Value) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::post(path)
                    .header("host", "127.0.0.1:3000")
                    .header("x-croaker-local", "1")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    assert_eq!(
        post(&app, "/local-api/bob/online", json!({"online":false}))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let read = json!({"conversation_id":chat,"targets":[{"message_id":target,"sender":"alice"}]});
    for _ in 0..2 {
        assert_eq!(
            post(&app, "/local-api/bob/read", read.clone())
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }
    drop(app);
    let mut workspace = Workspace::open(dir.path().into()).await.unwrap();
    workspace.snapshot("bob").await.unwrap();
    let state = workspace.snapshot("alice").await.unwrap();
    assert_eq!(
        find(&state, chat, target)["receipts"]["read"],
        json!(["bob"])
    );
    let app = demo_test_router(workspace, std::path::PathBuf::from("missing"), 3000);
    let response=post(&app,"/local-api/bob/read",json!({"conversation_id":chat,"targets":[{"message_id":target,"sender":"alice"}],"actor":"maya"})).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let _ = to_bytes(response.into_body(), 1024).await.unwrap();
}
