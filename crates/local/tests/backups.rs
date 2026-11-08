use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_client::backup::{self, RecoveryCode};
use croaker_local::{Attachment, Content, Workspace, demo_test_router as router};
use croaker_protocol::*;
use serde_json::json;
use std::path::PathBuf;
use tower::ServiceExt;

#[tokio::test]
async fn backup_is_portable_history_only_and_restores_without_changing_live_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(dir.path().into()).await.unwrap();
    let state = workspace.snapshot("alice").await.unwrap();
    let chat: ConversationId =
        serde_json::from_value(state["conversations"][0]["id"].clone()).unwrap();
    let target: MessageId =
        serde_json::from_value(state["conversations"][0]["messages"][0]["id"].clone()).unwrap();
    workspace
        .send(
            "alice",
            chat,
            MessageId::new(),
            Content {
                version: 1,
                text: "Backup-only marker 8172".into(),
                attachment: Some(Attachment {
                    name: "proof.txt".into(),
                    mime: "text/plain".into(),
                    data: BASE64_STANDARD.encode(b"attachment proof"),
                    size: 16,
                }),
            },
        )
        .await
        .unwrap();
    workspace
        .react(
            "alice",
            chat,
            MessageId::new(),
            target,
            "alice",
            Some("🐙".into()),
        )
        .await
        .unwrap();
    workspace.publish_typing("alice", chat, true).await.unwrap();
    // Bob's private conversation with Alice is not in Maya's export.
    let key = RecoveryCode::generate();
    let code = key.display();
    let maya = backup::decrypt(&workspace.export_backup("maya", &code).unwrap(), &key).unwrap();
    assert!(!maya.conversations.iter().any(|c| c.title == "Bob Chen"));
    let bytes = workspace.export_backup("alice", &code).unwrap();
    let archive = backup::decrypt(&bytes, &key).unwrap();
    assert!(
        archive
            .conversations
            .iter()
            .flat_map(|c| &c.messages)
            .any(|m| m.text == "Backup-only marker 8172"
                && m.attachment.as_ref().is_some_and(|f| f.size == 16))
    );
    assert!(
        archive
            .conversations
            .iter()
            .flat_map(|c| &c.messages)
            .any(|m| m.reactions.iter().any(|r| r.emoji == "🐙"))
    );
    let serialized = serde_json::to_string(&archive).unwrap();
    for secret_field in [
        "vault_key",
        "signer",
        "token",
        "ciphertext",
        "expires_at",
        "pending",
    ] {
        assert!(!serialized.contains(secret_field));
    }
    assert!(
        !bytes
            .windows(b"Backup-only marker".len())
            .any(|w| w == b"Backup-only marker")
    );
    let fresh = tempfile::tempdir().unwrap();
    let mut restored = Workspace::open(fresh.path().into()).await.unwrap();
    let before = restored.snapshot("bob").await.unwrap();
    let live_identities = before["profiles"].clone();
    let live_chats = before["conversations"].clone();
    assert!(
        restored
            .restore_backup("bob", &RecoveryCode::generate().display(), &bytes)
            .is_err()
    );
    assert!(restored.backup_archives("bob").unwrap().is_empty());
    let (id, imported) = restored.restore_backup("bob", &code, &bytes).unwrap();
    assert!(imported);
    assert!(!restored.restore_backup("bob", &code, &bytes).unwrap().1);
    assert!(restored.backup_archives("alice").unwrap().is_empty());
    let after = restored.snapshot("bob").await.unwrap();
    assert_eq!(after["profiles"], live_identities);
    assert_eq!(after["conversations"], live_chats);
    assert_eq!(restored.backup_archives("bob").unwrap()[0].id, id);
    drop(restored);
    let restored = Workspace::open(fresh.path().into()).await.unwrap();
    assert_eq!(restored.backup_archives("bob").unwrap().len(), 1);
    for entry in std::fs::read_dir(fresh.path()).unwrap().flatten() {
        let bytes = std::fs::read(entry.path()).unwrap();
        assert!(
            !bytes
                .windows(b"Backup-only marker".len())
                .any(|w| w == b"Backup-only marker")
        );
    }
}

#[tokio::test]
async fn backup_routes_preserve_origin_account_confirmation_and_size_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(directory.path().into()).await.unwrap();
    let initial = workspace.snapshot("alice").await.unwrap();
    let group: ConversationId =
        serde_json::from_value(initial["conversations"][0]["id"].clone()).unwrap();
    for _ in 0..2 {
        workspace
            .send(
                "alice",
                group,
                MessageId::new(),
                Content {
                    version: 1,
                    text: "Large backup fixture".into(),
                    attachment: Some(Attachment {
                        name: "large.bin".into(),
                        mime: "application/octet-stream".into(),
                        data: BASE64_STANDARD.encode(vec![42; 32 * 1024]),
                        size: 32 * 1024,
                    }),
                },
            )
            .await
            .unwrap();
    }
    let code = RecoveryCode::generate().display();
    let bytes = workspace.export_backup("alice", &code).unwrap();
    let app = router(workspace, PathBuf::from("missing-assets"), 3000);
    for (path, body, origin, status) in [
        (
            "/local-api/alice/backups/code",
            json!({}),
            "https://evil.example",
            StatusCode::FORBIDDEN,
        ),
        (
            "/local-api/missing/backups/code",
            json!({}),
            "http://127.0.0.1:3000",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/local-api/alice/backups/export",
            json!({"recovery_code":code.as_str(),"confirmation":"wrong"}),
            "http://127.0.0.1:3000",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/local-api/alice/backups/restore",
            json!({"recovery_code":code.as_str(),"backup":BASE64_STANDARD.encode(&bytes[..bytes.len()-1])}),
            "http://127.0.0.1:3000",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/local-api/alice/backups/restore",
            json!({"recovery_code":code.as_str(),"backup":BASE64_STANDARD.encode(&bytes)}),
            "http://127.0.0.1:3000",
            StatusCode::OK,
        ),
        (
            "/local-api/alice/backups/export",
            json!({"recovery_code":code.as_str(),"confirmation":code.as_str(),"extra":"x".repeat(2000)}),
            "http://127.0.0.1:3000",
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("host", "127.0.0.1:3000")
                    .header("origin", origin)
                    .header("x-croaker-local", "1")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert!(!body.windows(code.len()).any(|w| w == code.as_bytes()));
    }
    let response = app
        .oneshot(
            Request::get("/local-api/bob/backups/archives")
                .header("host", "127.0.0.1:3000")
                .header("x-croaker-local", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["archives"],
        json!([])
    );
}

#[tokio::test]
async fn offline_pending_actions_are_not_exported_or_flushed_by_backup() {
    let directory = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(directory.path().into()).await.unwrap();
    let initial = workspace.snapshot("alice").await.unwrap();
    let group = initial["conversations"][0]["id"].clone();
    let target = initial["conversations"][0]["messages"][0]["id"].clone();
    let app = router(workspace, PathBuf::from("missing-assets"), 3000);
    async fn post(
        app: axum::Router,
        path: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        app.oneshot(
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
        post(
            app.clone(),
            "/local-api/alice/online",
            json!({"online":false})
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(post(app.clone(),"/local-api/alice/send",json!({"conversation_id":group,"request_id":MessageId::new(),"content":{"version":1,"text":"Never export this unsent marker","attachment":null}})).await.status(),StatusCode::NO_CONTENT);
    assert_eq!(post(app.clone(),"/local-api/alice/reactions",json!({"conversation_id":group,"request_id":MessageId::new(),"target_message_id":target,"target_sender":"alice","emoji":"🐙"})).await.status(),StatusCode::NO_CONTENT);
    let key = RecoveryCode::generate();
    let code = key.display();
    let response = post(
        app.clone(),
        "/local-api/alice/backups/export",
        json!({"recovery_code":code.as_str(),"confirmation":code.as_str()}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), backup::MAX_BACKUP_BYTES)
        .await
        .unwrap();
    let archive = backup::decrypt(&bytes, &key).unwrap();
    assert!(
        !archive
            .conversations
            .iter()
            .flat_map(|c| &c.messages)
            .any(
                |m| m.text.contains("unsent marker") || m.reactions.iter().any(|r| r.emoji == "🐙")
            )
    );
    let response = app
        .oneshot(
            Request::get("/local-api/state?profile=alice")
                .header("host", "127.0.0.1:3000")
                .header("x-croaker-local", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let state: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(state["online"], false);
    assert!(
        state["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == "Never export this unsent marker" && m["pending"] == true)
    );
}

#[tokio::test]
async fn encrypted_backup_keeps_reply_references_without_receipt_control_rows() {
    let directory = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(directory.path().into()).await.unwrap();
    let initial = workspace.snapshot("bob").await.unwrap();
    let group = serde_json::from_value(initial["conversations"][0]["id"].clone()).unwrap();
    let target =
        serde_json::from_value(initial["conversations"][0]["messages"][0]["id"].clone()).unwrap();
    workspace
        .send_reply(
            "bob",
            group,
            MessageId::new(),
            Content {
                version: 1,
                text: "Archived reply body 94871".into(),
                attachment: None,
            },
            target,
            "alice",
        )
        .await
        .unwrap();
    let state = workspace.snapshot("bob").await.unwrap();
    let count = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|chat| chat["messages"].as_array().unwrap().len())
        .sum::<usize>();
    let key = RecoveryCode::generate();
    let encrypted = workspace.export_backup("bob", &key.display()).unwrap();
    let archive = backup::decrypt(&encrypted, &key).unwrap();
    assert_eq!(
        archive
            .conversations
            .iter()
            .map(|chat| chat.messages.len())
            .sum::<usize>(),
        count
    );
    let chat = archive
        .conversations
        .iter()
        .find(|chat| chat.id == group)
        .unwrap();
    let reply = chat
        .messages
        .iter()
        .find(|message| message.text == "Archived reply body 94871")
        .unwrap();
    assert_eq!(reply.reply_to.as_ref().unwrap().message_id, target);
    assert!(
        chat.messages.iter().any(|message| message.id == target
            && message.sender == reply.reply_to.as_ref().unwrap().sender)
    );
    assert!(
        !serde_json::to_string(&archive)
            .unwrap()
            .contains("\"kind\":\"receipt\"")
    );
}
