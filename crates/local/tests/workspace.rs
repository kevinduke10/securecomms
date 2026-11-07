use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use croaker_local::{Content, Workspace, router};
use croaker_protocol::*;
use std::path::PathBuf;
use tower::ServiceExt;

#[tokio::test]
async fn workspace_messages_are_encrypted_idempotent_and_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(dir.path().to_path_buf()).await.unwrap();
    let state = workspace.snapshot("alice").await.unwrap();
    let group: ConversationId =
        serde_json::from_value(state["conversations"][0]["id"].clone()).unwrap();
    let message = Content {
        version: 1,
        text: "Unique private history test 8912".into(),
        attachment: None,
    };
    let request = MessageId::new();
    workspace
        .send("alice", group, request, message.clone())
        .await
        .unwrap();
    workspace
        .send("alice", group, request, message)
        .await
        .unwrap();
    let state = workspace.snapshot("bob").await.unwrap();
    let count = state["conversations"][0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["text"] == "Unique private history test 8912")
        .count();
    assert_eq!(count, 1);
    assert!(
        workspace
            .create_chat(
                "maya",
                "Not allowed".into(),
                "group".into(),
                vec!["bob".into()]
            )
            .await
            .is_err()
    );
    drop(workspace);
    let mut workspace = Workspace::open(dir.path().to_path_buf()).await.unwrap();
    let state = workspace.snapshot("bob").await.unwrap();
    assert!(
        state["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == "Unique private history test 8912")
    );
    for file in std::fs::read_dir(dir.path()).unwrap().flatten() {
        let bytes = std::fs::read(file.path()).unwrap();
        assert!(
            !bytes
                .windows(b"Unique private history test 8912".len())
                .any(|w| w == b"Unique private history test 8912"),
            "plaintext persisted in {}",
            file.path().display()
        );
    }
}

#[tokio::test]
async fn local_api_rejects_cross_origin_and_rebinding_requests() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(dir.path().to_path_buf()).await.unwrap();
    let app = router(workspace, PathBuf::from("missing-assets"), 3000);
    for (host, origin, header, expected) in [
        (
            "127.0.0.1:3000",
            "http://127.0.0.1:3000",
            true,
            StatusCode::OK,
        ),
        (
            "127.0.0.1:3000",
            "https://untrusted.example",
            true,
            StatusCode::FORBIDDEN,
        ),
        (
            "untrusted.example:3000",
            "http://untrusted.example:3000",
            true,
            StatusCode::FORBIDDEN,
        ),
        (
            "127.0.0.1:3000",
            "http://127.0.0.1:3000",
            false,
            StatusCode::FORBIDDEN,
        ),
    ] {
        let mut request = Request::get("/local-api/state?profile=alice")
            .header("host", host)
            .header("origin", origin);
        if header {
            request = request.header("x-croaker-local", "1");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}
