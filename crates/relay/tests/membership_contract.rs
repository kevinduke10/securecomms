use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_protocol::{membership::*, *};
use croaker_relay::{Store, StoreError};
use std::sync::Arc;
fn event(kind: EventKind, recipient: Option<DeviceId>) -> SubmitEvent {
    SubmitEvent {
        protocol_version: 1,
        message_id: MessageId::new(),
        kind,
        recipient,
        ciphertext: BASE64_STANDARD.encode(b"opaque authenticated MLS framing fixture"),
    }
}
async fn contract(store: Arc<dyn Store>) {
    let tenant = TenantId::new();
    let owner = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Admin,
    };
    let member = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Member,
    };
    let added = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Member,
    };
    let outsider = Principal {
        tenant_id: TenantId::new(),
        device_id: DeviceId::new(),
        role: Role::Admin,
    };
    let principals = [&owner, &member, &added, &outsider];
    for actor in principals {
        store
            .provision(
                actor,
                &format!(
                    "{}{}",
                    MessageId::new().0.simple(),
                    MessageId::new().0.simple()
                ),
            )
            .await
            .unwrap();
    }
    let id = ConversationId::new();
    let original = store
        .create(
            &owner,
            CreateConversation {
                id,
                devices: vec![owner.device_id, member.device_id],
            },
        )
        .await
        .unwrap();
    let before = event(EventKind::Application, None);
    store.append(&owner, id, before.clone()).await.unwrap();
    assert!(store.sync(&added, id, 0, 32).await.is_err());
    let mut admission = GroupAdmission {
        request_id: MessageId::new(),
        expected_devices: original.devices.clone(),
        added_device: added.device_id,
        commit: event(EventKind::Commit, None),
        welcome: event(EventKind::Welcome, Some(added.device_id)),
    };
    assert!(matches!(
        store.admit(&member, id, admission.clone()).await,
        Err(StoreError::NotFound)
    ));
    assert!(store.admit(&outsider, id, admission.clone()).await.is_err());
    let mut stale = admission.clone();
    stale.expected_devices = vec![owner.device_id];
    assert!(matches!(
        store.admit(&owner, id, stale).await,
        Err(StoreError::Conflict)
    ));
    // Failure on second inserted event must roll back first event and roster.
    admission.welcome.message_id = before.message_id;
    assert!(matches!(
        store.admit(&owner, id, admission.clone()).await,
        Err(StoreError::Conflict)
    ));
    assert_eq!(store.conversation_info(&owner, id).await.unwrap(), original);
    assert_eq!(store.sync(&member, id, 0, 32).await.unwrap().next_cursor, 1);
    admission.welcome.message_id = MessageId::new();
    let accepted = store.admit(&owner, id, admission.clone()).await.unwrap();
    assert_eq!(accepted.commit_sequence, 2);
    assert_eq!(accepted.welcome_sequence, 3);
    assert!(!accepted.duplicate);
    let retry = store.admit(&owner, id, admission.clone()).await.unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.commit_sequence, accepted.commit_sequence);
    let mut changed = admission.clone();
    changed.commit.ciphertext = BASE64_STANDARD.encode(b"different ciphertext");
    assert!(matches!(
        store.admit(&owner, id, changed).await,
        Err(StoreError::Conflict)
    ));
    let newcomer = store.sync(&added, id, 0, 32).await.unwrap();
    assert_eq!(newcomer.events.len(), 1);
    assert_eq!(newcomer.events[0].event, admission.welcome);
    assert_eq!(newcomer.next_cursor, 3);
    let existing = store.sync(&member, id, 0, 32).await.unwrap();
    assert_eq!(existing.events.len(), 2);
    assert_eq!(existing.events[0].event, before);
    assert_eq!(existing.events[1].event, admission.commit);
    let after = event(EventKind::Application, None);
    store.append(&owner, id, after.clone()).await.unwrap();
    assert_eq!(
        store.sync(&added, id, 3, 32).await.unwrap().events[0].event,
        after
    );
    assert!(
        store
            .conversation_info(&owner, id)
            .await
            .unwrap()
            .devices
            .contains(&added.device_id)
    );
    store.revoke(&owner, added.device_id).await.unwrap();
    assert!(store.sync(&added, id, 0, 32).await.is_err());
    assert!(
        store
            .admit(&owner, id, admission.clone())
            .await
            .unwrap()
            .duplicate
    ); // No reactivation on retry.
    store.revoke(&owner, owner.device_id).await.unwrap();
    assert!(store.admit(&owner, id, admission).await.is_err());
}
#[tokio::test]
async fn sqlite_admission_contract() {
    contract(Arc::new(
        croaker_relay::sqlite::SqliteStore::open(":memory:").unwrap(),
    ))
    .await;
}
#[cfg(feature = "aws")]
#[tokio::test]
#[ignore = "requires DynamoDB Local; run scripts/test-dynamodb.sh"]
async fn dynamodb_admission_contract() {
    use aws_sdk_dynamodb::types::{
        AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType,
    };
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .endpoint_url(
            std::env::var("CROAKER_DYNAMODB_ENDPOINT").expect("set loopback emulator endpoint"),
        )
        .load()
        .await;
    let client = aws_sdk_dynamodb::Client::new(&config);
    let table = format!("cirra-membership-test-{}", MessageId::new());
    client
        .create_table()
        .table_name(&table)
        .billing_mode(BillingMode::PayPerRequest)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("pk")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .unwrap(),
        )
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("sk")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .unwrap(),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("pk")
                .key_type(KeyType::Hash)
                .build()
                .unwrap(),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("sk")
                .key_type(KeyType::Range)
                .build()
                .unwrap(),
        )
        .send()
        .await
        .unwrap();
    contract(Arc::new(croaker_relay::dynamodb::DynamoStore::new(
        client.clone(),
        table.clone(),
    )))
    .await;
    client
        .delete_table()
        .table_name(table)
        .send()
        .await
        .unwrap();
}
