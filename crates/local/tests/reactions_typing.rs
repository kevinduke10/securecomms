use croaker_local::{Content, Workspace};
use croaker_protocol::*;
use serde_json::Value;

fn id(value: &Value) -> MessageId {
    serde_json::from_value(value.clone()).unwrap()
}
fn chat_id(value: &Value) -> ConversationId {
    serde_json::from_value(value.clone()).unwrap()
}
fn first(state: &Value) -> &Value {
    &state["conversations"][0]
}
fn reactions(state: &Value) -> &Value {
    &first(state)["messages"][0]["reactions"]
}

#[tokio::test]
async fn reactions_sync_replace_remove_deduplicate_and_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(dir.path().to_path_buf()).await.unwrap();
    let initial = workspace.snapshot("alice").await.unwrap();
    let chat = chat_id(&first(&initial)["id"]);
    let target = id(&first(&initial)["messages"][0]["id"]);
    let count = first(&initial)["messages"].as_array().unwrap().len();
    let operation = MessageId::new();
    for _ in 0..2 {
        workspace
            .react("bob", chat, operation, target, "alice", Some("👍".into()))
            .await
            .unwrap();
    }
    let state = workspace.snapshot("alice").await.unwrap();
    assert_eq!(reactions(&state)[0]["users"], serde_json::json!(["bob"]));
    assert_eq!(first(&state)["messages"].as_array().unwrap().len(), count);
    workspace
        .react(
            "alice",
            chat,
            MessageId::new(),
            target,
            "alice",
            Some("👍".into()),
        )
        .await
        .unwrap();
    let state = workspace.snapshot("bob").await.unwrap();
    assert_eq!(reactions(&state)[0]["users"].as_array().unwrap().len(), 2);
    workspace
        .react(
            "bob",
            chat,
            MessageId::new(),
            target,
            "alice",
            Some("❤️".into()),
        )
        .await
        .unwrap();
    workspace
        .react("alice", chat, MessageId::new(), target, "alice", None)
        .await
        .unwrap();
    let state = workspace.snapshot("alice").await.unwrap();
    assert_eq!(reactions(&state).as_array().unwrap().len(), 1);
    assert_eq!(reactions(&state)[0]["emoji"], "❤️");
    assert_eq!(reactions(&state)[0]["users"], serde_json::json!(["bob"]));
    // Guests can react, but a removal changes only their own authenticated state.
    workspace
        .react(
            "maya",
            chat,
            MessageId::new(),
            target,
            "alice",
            Some("❤️".into()),
        )
        .await
        .unwrap();
    workspace
        .react("maya", chat, MessageId::new(), target, "alice", None)
        .await
        .unwrap();
    assert!(
        workspace
            .react(
                "bob",
                chat,
                MessageId::new(),
                target,
                "alice",
                Some("not an emoji".into())
            )
            .await
            .is_err()
    );
    assert!(
        workspace
            .react(
                "bob",
                chat,
                MessageId::new(),
                operation,
                "bob",
                Some("👍".into())
            )
            .await
            .is_err()
    );
    let other = chat_id(&initial["conversations"][1]["id"]);
    assert!(
        workspace
            .react(
                "bob",
                other,
                MessageId::new(),
                target,
                "alice",
                Some("👍".into())
            )
            .await
            .is_err()
    );
    assert!(
        workspace
            .react(
                "maya",
                other,
                MessageId::new(),
                target,
                "alice",
                Some("👍".into())
            )
            .await
            .is_err()
    );
    workspace
        .react(
            "bob",
            chat,
            MessageId::new(),
            target,
            "alice",
            Some("🐙".into()),
        )
        .await
        .unwrap();
    drop(workspace);
    let mut workspace = Workspace::open(dir.path().to_path_buf()).await.unwrap();
    let state = workspace.snapshot("alice").await.unwrap();
    assert_eq!(reactions(&state)[0]["users"], serde_json::json!(["bob"]));
    assert_eq!(reactions(&state)[0]["emoji"], "🐙");
    assert_eq!(first(&state)["messages"].as_array().unwrap().len(), count);
    for file in std::fs::read_dir(dir.path()).unwrap().flatten() {
        let bytes = std::fs::read(file.path()).unwrap();
        assert!(
            !bytes
                .windows(b"\"kind\":\"reaction\"".len())
                .any(|w| w == b"\"kind\":\"reaction\"")
        );
    }
}

#[tokio::test]
async fn typing_is_scoped_transient_and_cleared_by_stop_and_send() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(dir.path().to_path_buf()).await.unwrap();
    let initial = workspace.snapshot("alice").await.unwrap();
    let chat = chat_id(&first(&initial)["id"]);
    let count = first(&initial)["messages"].as_array().unwrap().len();
    workspace.publish_typing("alice", chat, true).await.unwrap();
    assert_eq!(
        first(&workspace.snapshot("bob").await.unwrap())["typing"],
        serde_json::json!(["alice"])
    );
    assert!(
        first(&workspace.snapshot("alice").await.unwrap())["typing"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        first(&workspace.snapshot("bob").await.unwrap())["messages"]
            .as_array()
            .unwrap()
            .len(),
        count
    );
    workspace
        .publish_typing("alice", chat, false)
        .await
        .unwrap();
    assert!(
        first(&workspace.snapshot("bob").await.unwrap())["typing"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    workspace.publish_typing("alice", chat, true).await.unwrap();
    workspace
        .send(
            "alice",
            chat,
            MessageId::new(),
            Content {
                version: 1,
                text: "Done composing.".into(),
                attachment: None,
            },
        )
        .await
        .unwrap();
    assert!(
        first(&workspace.snapshot("bob").await.unwrap())["typing"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let private = chat_id(&initial["conversations"][3]["id"]);
    workspace
        .publish_typing("maya", private, true)
        .await
        .unwrap();
    assert!(
        first(&workspace.snapshot("bob").await.unwrap())["typing"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    workspace.publish_typing("maya", chat, true).await.unwrap();
    assert_eq!(
        first(&workspace.snapshot("bob").await.unwrap())["typing"],
        serde_json::json!(["maya"])
    );
    workspace.publish_typing("bob", chat, true).await.unwrap();
    assert_eq!(
        first(&workspace.snapshot("alice").await.unwrap())["typing"],
        serde_json::json!(["bob", "maya"])
    );
    drop(workspace);
    let mut workspace = Workspace::open(dir.path().to_path_buf()).await.unwrap();
    let state = workspace.snapshot("bob").await.unwrap();
    assert!(first(&state)["typing"].as_array().unwrap().is_empty());
    assert_eq!(
        first(&state)["messages"].as_array().unwrap().len(),
        count + 1
    );
}
