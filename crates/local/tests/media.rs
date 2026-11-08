use croaker_local::{MediaFile, Workspace};
use croaker_protocol::*;
#[tokio::test]
async fn large_attachment_round_trip_retry_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path().into()).await.unwrap();
    let c = w
        .create_chat(
            "alice",
            "Media access test".into(),
            "group".into(),
            vec!["bob".into()],
        )
        .await
        .unwrap();
    let id = MessageId::new();
    let bytes = b"Confidential object content, not visible to relay. ".repeat(2048);
    w.send_media(
        "alice",
        c,
        id,
        MediaFile {
            name: "private.png",
            mime: "image/png",
            bytes: &bytes,
        },
        "A file",
        None,
    )
    .await
    .unwrap();
    w.send_media(
        "alice",
        c,
        id,
        MediaFile {
            name: "private.png",
            mime: "image/png",
            bytes: &bytes,
        },
        "A file",
        None,
    )
    .await
    .unwrap();
    let state = w.snapshot("bob").await.unwrap();
    let chat = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == c.to_string())
        .unwrap();
    let messages = chat["messages"].as_array().unwrap();
    let found: Vec<_> = messages
        .iter()
        .filter(|m| m["id"] == id.to_string())
        .collect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["media"]["name"], "private.png");
    assert!(found[0]["media"].get("key").is_none());
    assert!(found[0]["media"].get("sha256").is_none());
    let (_, _, plain) = w.download_media("bob", c, id, "alice").await.unwrap();
    assert_eq!(&*plain, &bytes);
    assert!(w.download_media("maya", c, id, "alice").await.is_err());
    drop(w);
    let mut w = Workspace::open(dir.path().into()).await.unwrap();
    w.snapshot("bob").await.unwrap();
    assert_eq!(
        &*w.download_media("bob", c, id, "alice").await.unwrap().2,
        &bytes
    );
    for file in std::fs::read_dir(dir.path().join("objects")).unwrap() {
        let data = std::fs::read(file.unwrap().path()).unwrap();
        assert!(!data.windows(20).any(|b| b == &bytes[..20]));
    }
}

#[tokio::test]
async fn compacted_missing_object_requires_new_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path().into()).await.unwrap();
    let c = w
        .create_chat(
            "alice",
            "Missing object".into(),
            "group".into(),
            vec!["bob".into()],
        )
        .await
        .unwrap();
    let id = MessageId::new();
    let bytes = vec![9; 65536];
    w.send_media(
        "alice",
        c,
        id,
        MediaFile {
            name: "file.bin",
            mime: "",
            bytes: &bytes,
        },
        "",
        None,
    )
    .await
    .unwrap();
    for entry in std::fs::read_dir(dir.path().join("objects")).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let error = w
        .send_media(
            "alice",
            c,
            id,
            MediaFile {
                name: "file.bin",
                mime: "",
                bytes: &bytes,
            },
            "",
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("choose a new attachment request")
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("objects"))
            .unwrap()
            .count(),
        0
    );
}
