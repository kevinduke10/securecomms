use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_protocol::{lifecycle::*, *};
use croaker_relay::{Store, StoreError};
use std::sync::Arc;
async fn contract(store: Arc<dyn Store>) {
    let tenant = TenantId::new();
    let owner = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Admin,
    };
    let member = Principal {
        device_id: DeviceId::new(),
        role: Role::Member,
        ..owner.clone()
    };
    let third = Principal {
        device_id: DeviceId::new(),
        role: Role::Member,
        ..owner.clone()
    };
    for p in [&owner, &member, &third] {
        store
            .provision(
                p,
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
                devices: vec![owner.device_id, member.device_id, third.device_id],
            },
        )
        .await
        .unwrap();
    let request = MessageId::new();
    let input = GroupLifecycle {
        context: LifecycleContext {
            conversation_id: id,
            request_id: request,
            expected_owner: owner.device_id,
            expected_devices: original.devices,
            action: LifecycleAction::Transfer {
                target: member.device_id,
            },
        },
        commit: SubmitEvent {
            protocol_version: API_VERSION,
            message_id: request,
            kind: EventKind::Commit,
            recipient: None,
            ciphertext: BASE64_STANDARD.encode(b"opaque MLS fixture"),
        },
    };
    assert!(matches!(
        store.lifecycle(&third, id, input.clone()).await,
        Err(StoreError::NotFound)
    ));
    let accepted = store.lifecycle(&owner, id, input.clone()).await.unwrap();
    assert_eq!(accepted.sequence, 1);
    assert!(
        store
            .lifecycle(&owner, id, input.clone())
            .await
            .unwrap()
            .duplicate
    );
    assert_eq!(
        store.conversation_info(&owner, id).await.unwrap().owner,
        member.device_id
    );
    let mut changed = input.clone();
    changed.commit.ciphertext = BASE64_STANDARD.encode(b"different");
    assert!(matches!(
        store.lifecycle(&owner, id, changed).await,
        Err(StoreError::Conflict)
    ));
    let request = MessageId::new();
    let mut remove = input;
    remove.context.request_id = request;
    remove.commit.message_id = request;
    remove.context.expected_owner = member.device_id;
    remove.context.action = LifecycleAction::Remove {
        target: third.device_id,
    };
    assert!(store.lifecycle(&owner, id, remove.clone()).await.is_err());
    store.lifecycle(&member, id, remove.clone()).await.unwrap();
    assert!(
        store
            .lifecycle(&member, id, remove)
            .await
            .unwrap()
            .duplicate
    );
    assert!(store.sync(&third, id, 0, 20).await.is_err());
    assert!(store.conversation_info(&third, id).await.is_err());
    let event = SubmitEvent {
        protocol_version: API_VERSION,
        message_id: MessageId::new(),
        kind: EventKind::Application,
        recipient: None,
        ciphertext: BASE64_STANDARD.encode(b"future ciphertext"),
    };
    assert!(store.append(&third, id, event.clone()).await.is_err());
    store.append(&member, id, event).await.unwrap();
    assert_eq!(store.sync(&owner, id, 0, 20).await.unwrap().next_cursor, 3);
}
#[cfg(feature = "local")]
#[tokio::test]
async fn sqlite_lifecycle_contract() {
    contract(Arc::new(
        croaker_relay::sqlite::SqliteStore::open(":memory:").unwrap(),
    ))
    .await;
}
#[cfg(feature = "aws")]
#[tokio::test]
#[ignore = "requires DynamoDB Local; run scripts/test-dynamodb.sh"]
async fn dynamodb_lifecycle_contract() {
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
    let table = format!("cirra-lifecycle-test-{}", MessageId::new());
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
