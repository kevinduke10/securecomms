use croaker_client::{ClientStore, backup::*};
use croaker_crypto::Device;
use croaker_protocol::{ConversationId, DeviceId, MessageId, TenantId};
use zeroize::Zeroizing;

fn archive() -> Archive {
    let source = DeviceId::new();
    Archive {
        version: 1,
        id: MessageId::new(),
        created_at: 1_800_000_000,
        tenant_id: TenantId::new(),
        source_device: source,
        people: vec![Person {
            device_id: source,
            name: "Private Person".into(),
        }],
        conversations: vec![Conversation {
            id: ConversationId::new(),
            title: "Secret group".into(),
            kind: "group".into(),
            members: vec![source],
            messages: vec![Message {
                reply_to: None,
                media_omitted: None,
                id: MessageId::new(),
                sender: source,
                sent_at: 1_800_000_000,
                text: "Private archive marker 982834".into(),
                attachment: Some(Attachment {
                    name: "private.txt".into(),
                    mime: "text/plain".into(),
                    data: "c2VjcmV0".into(),
                    size: 6,
                }),
                reactions: vec![Reaction {
                    sender: source,
                    emoji: "🐙".into(),
                }],
            }],
        }],
    }
}
#[test]
fn recovery_code_roundtrip_and_checksum_reject_mistakes() {
    let code = RecoveryCode::generate().display();
    assert_eq!(
        *code,
        *RecoveryCode::parse(&code.to_uppercase()).unwrap().display()
    );
    let mut typo = code.to_string().into_bytes();
    typo[5] = if typo[5] == b'0' { b'1' } else { b'0' };
    assert!(RecoveryCode::parse(std::str::from_utf8(&typo).unwrap()).is_err());
    for invalid in ["", "password", "CRK1-00000000", &"x".repeat(1000)] {
        assert!(RecoveryCode::parse(invalid).is_err());
    }
}
#[test]
fn randomized_encryption_authenticates_all_regions_and_hides_contents() {
    let source = archive();
    let key = RecoveryCode::generate();
    let bytes = encrypt(&source, &key).unwrap();
    assert_ne!(bytes, encrypt(&source, &key).unwrap());
    let restored = decrypt(&bytes, &key).unwrap();
    assert_eq!(
        serde_json::to_vec(&source).unwrap(),
        serde_json::to_vec(&restored).unwrap()
    );
    for marker in [
        "Private Person",
        "Secret group",
        "Private archive marker",
        "private.txt",
        "c2VjcmV0",
    ] {
        assert!(!bytes.windows(marker.len()).any(|w| w == marker.as_bytes()));
    }
    assert!(decrypt(&bytes, &RecoveryCode::generate()).is_err());
    // Version/header, salt, wrapped key nonce/tag, payload nonce/body/tag.
    for position in [0, 18, 49, 65, 100, 121, 140, bytes.len() - 1] {
        let mut damaged = bytes.clone();
        damaged[position] ^= 1;
        assert!(decrypt(&damaged, &key).is_err());
    }
    for length in [0, 1, 90, bytes.len() - 1] {
        assert!(decrypt(&bytes[..length], &key).is_err());
    }
    let mut appended = bytes.clone();
    appended.push(0);
    assert!(decrypt(&appended, &key).is_err());
    assert!(decrypt(&vec![0; MAX_BACKUP_BYTES + 1], &key).is_err());
}
#[test]
fn restore_is_atomic_idempotent_and_does_not_restore_live_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fresh.sqlite");
    let tenant = TenantId::new();
    let device_id = DeviceId::new();
    let device = Device::new(tenant, device_id).unwrap();
    let mut store = ClientStore::create(&path, device, Zeroizing::new([91; 32])).unwrap();
    let source = archive();
    let before = store.update(|device| Ok(device.binding().clone())).unwrap();
    let live = ConversationId::new();
    store
        .update(|device| device.create_conversation(live))
        .unwrap();
    let pending = store
        .queue_message(live, b"Existing live pending send")
        .unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let before_state: Vec<u8> = connection
        .query_row("SELECT sealed FROM state", [], |row| row.get(0))
        .unwrap();
    assert!(store.import_archive(&source).unwrap());
    let after_state: Vec<u8> = connection
        .query_row("SELECT sealed FROM state", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before_state, after_state);
    assert_eq!(store.pending().unwrap(), vec![pending]);
    assert!(!store.import_archive(&source).unwrap());
    assert!(
        store
            .history(source.conversations[0].id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.cursor(source.conversations[0].id).unwrap(), 0);
    assert_eq!(
        store.update(|device| Ok(device.binding().clone())).unwrap(),
        before
    );
    let mut conflict = source.clone();
    conflict.conversations[0].messages[0].text = "changed content under same archive ID".into();
    assert!(store.import_archive(&conflict).is_err());
    let mut invalid = source.clone();
    invalid.id = MessageId::new();
    invalid.conversations[0].members.clear();
    assert!(store.import_archive(&invalid).is_err());
    assert_eq!(store.recovered_archives().unwrap().len(), 1);
    drop(store);
    let mut store = ClientStore::open(&path, Zeroizing::new([91; 32])).unwrap();
    assert_eq!(
        store.recovered_archives().unwrap()[0].conversations[0].messages[0].text,
        "Private archive marker 982834"
    );
    for entry in std::fs::read_dir(directory.path()).unwrap().flatten() {
        let data = std::fs::read(entry.path()).unwrap();
        assert!(
            !data
                .windows(b"Private archive marker".len())
                .any(|w| w == b"Private archive marker")
        );
    }
    store.delete_archive(source.id).unwrap();
    assert!(store.recovered_archives().unwrap().is_empty());
}
#[test]
fn archive_validation_rejects_unknown_fields_duplicate_ids_and_bad_attachments() {
    let mut value = serde_json::to_value(archive()).unwrap();
    value["vault_key"] = "must never be accepted".into();
    assert!(serde_json::from_value::<Archive>(value).is_err());
    let mut source = archive();
    source.version = 2;
    assert!(source.validate().is_err());
    source.version = 1;
    source.people.push(source.people[0].clone());
    assert!(source.validate().is_err());
    source.people.pop();
    source.conversations[0].messages[0]
        .attachment
        .as_mut()
        .unwrap()
        .name = "../escape.txt".into();
    assert!(source.validate().is_err());
    source.conversations[0].messages[0]
        .attachment
        .as_mut()
        .unwrap()
        .name = "valid.txt".into();
    let duplicate = source.conversations[0].messages[0].clone();
    source.conversations[0].messages.push(duplicate);
    assert!(source.validate().is_err());
}

#[test]
fn export_stops_at_the_documented_archive_size_limit() {
    let mut source = archive();
    let template = source.conversations[0].messages[0].clone();
    source.conversations[0].messages = (0..2200)
        .map(|_| Message {
            reply_to: None,
            media_omitted: None,
            id: MessageId::new(),
            sender: template.sender,
            sent_at: template.sent_at,
            text: "x".repeat(8000),
            attachment: None,
            reactions: vec![],
        })
        .collect();
    assert!(source.validate().is_ok());
    assert!(matches!(
        encrypt(&source, &RecoveryCode::generate()),
        Err(BackupError::Size)
    ));
}

#[test]
fn history_archives_preserve_reply_references_and_accept_older_messages() {
    let mut source = archive();
    let first = source.conversations[0].messages[0].clone();
    let mut reply = first.clone();
    reply.id = MessageId::new();
    reply.text = "Reply body".into();
    reply.reply_to = Some(MessageRef {
        message_id: first.id,
        sender: first.sender,
    });
    source.conversations[0].messages.push(reply);
    let code = RecoveryCode::generate();
    let restored = decrypt(&encrypt(&source, &code).unwrap(), &code).unwrap();
    let reference = restored.conversations[0].messages[1]
        .reply_to
        .as_ref()
        .unwrap();
    assert_eq!(reference.message_id, first.id);
    assert_eq!(reference.sender, first.sender);
    let legacy = serde_json::to_value(archive()).unwrap();
    assert!(
        legacy["conversations"][0]["messages"][0]
            .get("reply_to")
            .is_none()
    );
    assert!(
        serde_json::from_value::<Archive>(legacy)
            .unwrap()
            .conversations[0]
            .messages[0]
            .reply_to
            .is_none()
    );
    source.conversations[0].messages[1].reply_to = Some(MessageRef {
        message_id: source.conversations[0].messages[1].id,
        sender: first.sender,
    });
    assert!(source.validate().is_err());
}

#[test]
fn object_media_backup_retains_only_validated_omission_metadata() {
    let mut source = archive();
    let message = &mut source.conversations[0].messages[0];
    message.text.clear();
    message.attachment = None;
    message.media_omitted = Some(OmittedMedia {
        name: "clip.mp4".into(),
        mime: "video/mp4".into(),
        size: 24 * 1024 * 1024,
    });
    source.validate().unwrap();
    let code = RecoveryCode::generate();
    let restored = decrypt(&encrypt(&source, &code).unwrap(), &code).unwrap();
    assert_eq!(
        restored.conversations[0].messages[0]
            .media_omitted
            .as_ref()
            .unwrap()
            .name,
        "clip.mp4"
    );
    let serialized = serde_json::to_value(&restored).unwrap();
    let media = &serialized["conversations"][0]["messages"][0]["media_omitted"];
    assert_eq!(media.as_object().unwrap().len(), 3);
    for forbidden in ["key", "object_id", "sha256", "ciphertext_size", "url"] {
        assert!(media.get(forbidden).is_none());
    }
    let mut injected = serialized.clone();
    injected["conversations"][0]["messages"][0]["media_omitted"]["key"] =
        serde_json::json!("secret");
    assert!(serde_json::from_value::<Archive>(injected).is_err());
    source.conversations[0].messages[0]
        .media_omitted
        .as_mut()
        .unwrap()
        .name = "../clip.mp4".into();
    assert!(source.validate().is_err());
    let legacy = serde_json::to_value(archive()).unwrap();
    assert!(
        legacy["conversations"][0]["messages"][0]
            .get("media_omitted")
            .is_none()
    );
    assert!(
        serde_json::from_value::<Archive>(legacy)
            .unwrap()
            .conversations[0]
            .messages[0]
            .media_omitted
            .is_none()
    );
}
