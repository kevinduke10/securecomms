use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_protocol::*;
use croaker_relay::{Store, StoreError, sqlite::SqliteStore};
use std::sync::Arc;

fn actor(tenant: TenantId, role: Role) -> Principal {
    Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role,
    }
}
fn event() -> SubmitEvent {
    SubmitEvent {
        protocol_version: API_VERSION,
        message_id: MessageId::new(),
        kind: EventKind::Application,
        recipient: None,
        ciphertext: BASE64_STANDARD.encode(b"opaque encrypted test payload"),
    }
}

async fn contract(store: Arc<dyn Store>) {
    let tenant = TenantId::new();
    let alice = actor(tenant, Role::Admin);
    let bob = actor(tenant, Role::Member);
    let guest = actor(tenant, Role::Guest);
    let same_tenant_outsider = actor(tenant, Role::Member);
    let outsider = actor(TenantId::new(), Role::Admin);
    let tokens: Vec<_> = (0..5)
        .map(|_| {
            format!(
                "{}{}",
                MessageId::new().0.simple(),
                MessageId::new().0.simple()
            )
        })
        .collect();
    for (principal, token) in [&alice, &bob, &guest, &same_tenant_outsider, &outsider]
        .into_iter()
        .zip(&tokens)
    {
        store.provision(principal, token).await.unwrap();
        assert_eq!(&store.authenticate(token).await.unwrap(), principal);
        assert!(matches!(
            store.provision(principal, token).await,
            Err(StoreError::Conflict)
        ));
    }
    assert!(matches!(
        store.authenticate(&"0".repeat(64)).await,
        Err(StoreError::Unauthorized)
    ));
    let id = ConversationId::new();
    let create = CreateConversation {
        id,
        devices: vec![alice.device_id, bob.device_id, guest.device_id],
    };
    let group = store.create(&alice, create.clone()).await.unwrap();
    assert_eq!(group, store.create(&alice, create.clone()).await.unwrap());
    assert!(matches!(
        store
            .create(
                &guest,
                CreateConversation {
                    id: ConversationId::new(),
                    devices: vec![guest.device_id, alice.device_id]
                }
            )
            .await,
        Err(StoreError::NotFound)
    ));
    assert!(
        store
            .create(
                &alice,
                CreateConversation {
                    id: ConversationId::new(),
                    devices: vec![alice.device_id, outsider.device_id]
                }
            )
            .await
            .is_err()
    );
    for attacker in [&outsider, &same_tenant_outsider] {
        assert!(matches!(
            store.sync(attacker, id, 0, 10).await,
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            store.append(attacker, id, event()).await,
            Err(StoreError::NotFound)
        ));
    }
    assert!(matches!(
        store.revoke(&outsider, bob.device_id).await,
        Err(StoreError::NotFound)
    ));
    let message = event();
    let first = store.append(&alice, id, message.clone()).await.unwrap();
    let retry = store.append(&alice, id, message.clone()).await.unwrap();
    assert_eq!(first.sequence, retry.sequence);
    assert!(!first.duplicate && retry.duplicate);
    let mut conflict = message;
    conflict.ciphertext = BASE64_STANDARD.encode(b"different payload");
    assert!(matches!(
        store.append(&alice, id, conflict).await,
        Err(StoreError::Conflict)
    ));
    let mut invalid = event();
    invalid.protocol_version = 2;
    assert!(matches!(
        store.append(&alice, id, invalid).await,
        Err(StoreError::Invalid(_))
    ));
    let mut invalid = event();
    invalid.ciphertext = BASE64_STANDARD.encode(vec![0; MAX_CIPHERTEXT_BYTES + 1]);
    assert!(matches!(
        store.append(&alice, id, invalid).await,
        Err(StoreError::Invalid(_))
    ));

    // Private welcomes are never returned to other members, and invisible
    // events still advance the sync cursor to prevent pagination loops.
    let mut welcome = event();
    welcome.kind = EventKind::Welcome;
    welcome.recipient = Some(bob.device_id);
    let welcome = store.append(&alice, id, welcome).await.unwrap();
    let page = store.sync(&guest, id, first.sequence, 1).await.unwrap();
    assert!(page.events.is_empty());
    assert_eq!(page.next_cursor, welcome.sequence);

    let mut commits = event();
    commits.kind = EventKind::Commit;
    assert!(matches!(
        store.append(&bob, id, commits).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.revoke(&bob, alice.device_id).await,
        Err(StoreError::NotFound)
    ));
    assert!(store.sync(&bob, id, 0, 0).await.is_err());
    assert!(store.sync(&bob, id, u64::MAX, 10).await.is_err());

    let mut tasks = Vec::new();
    for _ in 0..4 {
        let store = store.clone();
        let alice = alice.clone();
        tasks.push(tokio::spawn(async move {
            store.append(&alice, id, event()).await.unwrap().sequence
        }));
    }
    let mut sequences = Vec::new();
    for task in tasks {
        sequences.push(task.await.unwrap());
    }
    sequences.sort();
    assert_eq!(sequences, vec![3, 4, 5, 6]);
    let all = store.sync(&bob, id, 0, MAX_SYNC_PAGE).await.unwrap();
    assert_eq!(all.events.len(), 6);
    assert!(!all.has_more);
    let mut cursor = 0;
    let mut seen = Vec::new();
    loop {
        let page = store.sync(&bob, id, cursor, 2).await.unwrap();
        assert!(page.next_cursor >= cursor);
        cursor = page.next_cursor;
        seen.extend(page.events.into_iter().map(|e| e.sequence));
        if !page.has_more {
            break;
        }
    }
    assert_eq!(seen, vec![1, 2, 3, 4, 5, 6]);

    store.revoke(&alice, bob.device_id).await.unwrap();
    assert!(matches!(
        store.authenticate(&tokens[1]).await,
        Err(StoreError::Unauthorized)
    ));
    assert!(matches!(
        store.sync(&bob, id, 0, MAX_SYNC_PAGE).await,
        Err(StoreError::Unauthorized)
    ));
    assert!(matches!(
        store.append(&bob, id, event()).await,
        Err(StoreError::Unauthorized)
    ));
    assert!(store.sync(&guest, id, 0, MAX_SYNC_PAGE).await.is_ok());
    assert!(store.sync(&outsider, id, 0, MAX_SYNC_PAGE).await.is_err());
    store.revoke(&guest, guest.device_id).await.unwrap();
    assert!(matches!(
        store.authenticate(&tokens[2]).await,
        Err(StoreError::Unauthorized)
    ));
}

#[tokio::test]
async fn sqlite_store_contract() {
    contract(Arc::new(SqliteStore::open(":memory:").unwrap())).await;
}

#[tokio::test]
async fn sqlite_reopen_preserves_messages_and_deduplication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.sqlite");
    let alice = actor(TenantId::new(), Role::Admin);
    let bob = actor(alice.tenant_id, Role::Member);
    let id = ConversationId::new();
    let message = event();
    {
        let store = SqliteStore::open(&path).unwrap();
        store.provision(&alice, &"a".repeat(64)).await.unwrap();
        store.provision(&bob, &"b".repeat(64)).await.unwrap();
        store
            .create(
                &alice,
                CreateConversation {
                    id,
                    devices: vec![alice.device_id, bob.device_id],
                },
            )
            .await
            .unwrap();
        store.append(&alice, id, message.clone()).await.unwrap();
    }
    let store = SqliteStore::open(path).unwrap();
    assert_eq!(store.sync(&bob, id, 0, 10).await.unwrap().events.len(), 1);
    assert!(store.append(&alice, id, message).await.unwrap().duplicate);
}

#[cfg(feature = "aws")]
#[tokio::test]
#[ignore = "requires DynamoDB Local; run scripts/test-dynamodb.sh"]
async fn dynamodb_store_contract() {
    use aws_sdk_dynamodb::{
        Client,
        types::{AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType},
    };
    let endpoint = std::env::var("CROAKER_DYNAMODB_ENDPOINT").expect("local DynamoDB endpoint");
    assert!(
        endpoint.starts_with("http://127.0.0.1:"),
        "integration test only supports loopback"
    );
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new("us-east-1"))
        .endpoint_url(endpoint)
        .load()
        .await;
    let client = Client::new(&config);
    let table = format!("croaker-test-{}", MessageId::new());
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
    let store = croaker_relay::dynamodb::DynamoStore::new(client.clone(), table.clone());
    contract(Arc::new(store)).await;
    client
        .delete_table()
        .table_name(table)
        .send()
        .await
        .unwrap();
}

#[tokio::test]
async fn expiration_scrubs_local_payload_and_preserves_retry_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay.sqlite");
    let store = SqliteStore::open(&path).unwrap();
    let alice = actor(TenantId::new(), Role::Admin);
    let bob = actor(alice.tenant_id, Role::Member);
    store.provision(&alice, &"d".repeat(64)).await.unwrap();
    store.provision(&bob, &"e".repeat(64)).await.unwrap();
    let id = ConversationId::new();
    store
        .create(
            &alice,
            CreateConversation {
                id,
                devices: vec![alice.device_id, bob.device_id],
            },
        )
        .await
        .unwrap();
    let message = event();
    let original = store.append(&alice, id, message.clone()).await.unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("UPDATE events SET expires_at=0", [])
        .unwrap();
    let page = store.sync(&bob, id, 0, 10).await.unwrap();
    assert!(page.events.is_empty());
    assert_eq!(page.next_cursor, original.sequence);
    let payload: Option<String> = connection
        .query_row("SELECT payload FROM events", [], |r| r.get(0))
        .unwrap();
    assert!(payload.is_none());
    let retry = store.append(&alice, id, message).await.unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.sequence, original.sequence);
}
