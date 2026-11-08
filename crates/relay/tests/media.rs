#![cfg(feature = "local")]
use croaker_protocol::{media::*, *};
use croaker_relay::{
    StoreError,
    media::{
        Objects, Registry,
        local::{Files, SqliteRegistry},
    },
};
use sha2::{Digest, Sha256};
#[tokio::test]
async fn immutable_scoped_idempotent_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let db = SqliteRegistry::open(dir.path().join("media.db")).unwrap();
    let files = Files::open(dir.path().join("objects"), "http://127.0.0.1:9999".into()).unwrap();
    let actor = Principal {
        tenant_id: TenantId::new(),
        device_id: DeviceId::new(),
        role: Role::Member,
    };
    let c = ConversationId::new();
    let bytes = vec![37; 64 * 1024];
    let u = ObjectUpload {
        object_id: MessageId::new(),
        ciphertext_size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(&bytes)),
    };
    let r = db.reserve(&actor, c, u.clone()).await.unwrap();
    assert!(!files.upload(&r).await.unwrap().already_uploaded);
    files.put(&r, &bytes).await.unwrap();
    assert!(files.upload(&r).await.unwrap().already_uploaded);
    assert_eq!(files.read(&r).await.unwrap(), bytes);
    files.put(&r, &bytes).await.unwrap();
    assert!(files.put(&r, &vec![0; bytes.len()]).await.is_err());
    let mut bad = u.clone();
    bad.sha256 = "0".repeat(64);
    assert!(matches!(
        db.reserve(&actor, c, bad).await,
        Err(StoreError::Conflict)
    ));
    let mut other = actor.clone();
    other.device_id = DeviceId::new();
    assert!(db.reserve(&other, c, u.clone()).await.is_err());
    assert!(db.find(TenantId::new(), c, u.object_id).await.is_err());
    let reopened = SqliteRegistry::open(dir.path().join("media.db")).unwrap();
    assert_eq!(
        reopened
            .find(actor.tenant_id, c, u.object_id)
            .await
            .unwrap()
            .upload,
        u
    );
}
#[tokio::test]
async fn quota_counts_reservations_not_retries() {
    let dir = tempfile::tempdir().unwrap();
    let db = SqliteRegistry::open(dir.path().join("m")).unwrap();
    let a = Principal {
        tenant_id: TenantId::new(),
        device_id: DeviceId::new(),
        role: Role::Member,
    };
    let c = ConversationId::new();
    for _ in 0..9 {
        let u = ObjectUpload {
            object_id: MessageId::new(),
            ciphertext_size: (MAX_MEDIA_BYTES + MEDIA_OVERHEAD) as u64,
            sha256: "0".repeat(64),
        };
        db.reserve(&a, c, u.clone()).await.unwrap();
        db.reserve(&a, c, u).await.unwrap();
    }
    let u = ObjectUpload {
        object_id: MessageId::new(),
        ciphertext_size: (MAX_MEDIA_BYTES + MEDIA_OVERHEAD) as u64,
        sha256: "0".repeat(64),
    };
    assert!(db.reserve(&a, c, u).await.is_err());
}
#[tokio::test]
async fn current_membership_and_revocation_gate_every_object_access() {
    use croaker_relay::{Store, media::ObjectService, sqlite::SqliteStore};
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let relay = Arc::new(SqliteStore::open(":memory:").unwrap());
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
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Member,
    };
    let foreign = Principal {
        tenant_id: TenantId::new(),
        device_id: DeviceId::new(),
        role: Role::Admin,
    };
    for (a, t) in [
        (&alice, "a"),
        (&bob, "b"),
        (&outsider, "c"),
        (&foreign, "d"),
    ] {
        relay.provision(a, &t.repeat(64)).await.unwrap();
    }
    let c = ConversationId::new();
    relay
        .create(
            &alice,
            CreateConversation {
                id: c,
                devices: vec![alice.device_id, bob.device_id],
            },
        )
        .await
        .unwrap();
    let service = ObjectService {
        relay: relay.clone(),
        registry: Arc::new(SqliteRegistry::open(dir.path().join("m")).unwrap()),
        objects: Arc::new(Files::open(dir.path().join("o"), "".into()).unwrap()),
    };
    let bytes = vec![42; 65536];
    let u = ObjectUpload {
        object_id: MessageId::new(),
        ciphertext_size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(&bytes)),
    };
    for a in [&outsider, &foreign] {
        assert!(service.reserve(a, c, u.clone()).await.is_err());
    }
    service.put(&alice, c, u.clone(), &bytes).await.unwrap();
    assert_eq!(service.read(&bob, c, u.object_id).await.unwrap(), bytes);
    for a in [&outsider, &foreign] {
        assert!(service.read(a, c, u.object_id).await.is_err());
    }
    relay.revoke(&alice, bob.device_id).await.unwrap();
    assert!(service.read(&bob, c, u.object_id).await.is_err());
    assert!(
        service
            .reserve(
                &bob,
                c,
                ObjectUpload {
                    object_id: MessageId::new(),
                    ..u
                }
            )
            .await
            .is_err()
    );
}

#[cfg(feature = "aws")]
#[tokio::test]
#[ignore = "requires DynamoDB Local; run scripts/test-dynamodb.sh"]
async fn dynamodb_media_contract() {
    use aws_sdk_dynamodb::{
        Client,
        types::{AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType},
    };
    use croaker_relay::media::dynamodb::DynamoRegistry;
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
    let table = format!("cirra-media-test-{}", MessageId::new());
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
    let db = DynamoRegistry::new(client.clone(), table.clone());
    let a = Principal {
        tenant_id: TenantId::new(),
        device_id: DeviceId::new(),
        role: Role::Member,
    };
    let c = ConversationId::new();
    let original = ObjectUpload {
        object_id: MessageId::new(),
        ciphertext_size: 65536,
        sha256: "1".repeat(64),
    };
    let saved = db.reserve(&a, c, original.clone()).await.unwrap();
    let same = db.reserve(&a, c, original.clone()).await.unwrap();
    assert_eq!(same.expires_at, saved.expires_at);
    assert_eq!(same.upload, original);
    let mut conflict = original.clone();
    conflict.sha256 = "2".repeat(64);
    assert!(matches!(
        db.reserve(&a, c, conflict).await,
        Err(StoreError::Conflict)
    ));
    let mut other = a.clone();
    other.device_id = DeviceId::new();
    assert!(matches!(
        db.reserve(&other, c, original.clone()).await,
        Err(StoreError::Conflict)
    ));
    assert!(matches!(
        db.find(TenantId::new(), c, original.object_id).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        db.find(a.tenant_id, ConversationId::new(), original.object_id)
            .await,
        Err(StoreError::NotFound)
    ));
    // Repeated reservations must not charge quotas again; failed transactions must neither
    // consume bytes nor insert a mutable object. This exercises both conditional updates.
    for _ in 0..9 {
        let u = ObjectUpload {
            object_id: MessageId::new(),
            ciphertext_size: (MAX_MEDIA_BYTES + MEDIA_OVERHEAD) as u64,
            sha256: "0".repeat(64),
        };
        db.reserve(&a, c, u.clone()).await.unwrap();
        db.reserve(&a, c, u).await.unwrap();
    }
    let too_large = ObjectUpload {
        object_id: MessageId::new(),
        ciphertext_size: (MAX_MEDIA_BYTES + MEDIA_OVERHEAD) as u64,
        sha256: "0".repeat(64),
    };
    assert!(matches!(
        db.reserve(&a, c, too_large.clone()).await,
        Err(StoreError::Invalid("daily media upload limit reached"))
    ));
    assert!(matches!(
        db.find(a.tenant_id, c, too_large.object_id).await,
        Err(StoreError::NotFound)
    ));
    let small = ObjectUpload {
        object_id: MessageId::new(),
        ciphertext_size: 1024 * 1024,
        sha256: "0".repeat(64),
    };
    db.reserve(&a, c, small).await.unwrap();
    // Exhaust the tenant budget with distinct devices, each below its own budget.
    let mut denied = false;
    for _ in 0..5 {
        let mut device = a.clone();
        device.device_id = DeviceId::new();
        for _ in 0..9 {
            let u = ObjectUpload {
                object_id: MessageId::new(),
                ciphertext_size: (MAX_MEDIA_BYTES + MEDIA_OVERHEAD) as u64,
                sha256: "0".repeat(64),
            };
            match db.reserve(&device, c, u).await {
                Ok(_) => {}
                Err(StoreError::Invalid("daily media upload limit reached")) => {
                    denied = true;
                    break;
                }
                Err(e) => panic!("unexpected reservation failure: {e}"),
            }
        }
        if denied {
            break;
        }
    }
    assert!(denied, "tenant quota must be shared across devices");
    assert_eq!(
        db.find(a.tenant_id, c, original.object_id)
            .await
            .unwrap()
            .upload,
        original
    );
    client
        .delete_table()
        .table_name(&table)
        .send()
        .await
        .unwrap();
    assert!(matches!(
        db.reserve(
            &a,
            c,
            ObjectUpload {
                object_id: MessageId::new(),
                ciphertext_size: 65536,
                sha256: "0".repeat(64)
            }
        )
        .await,
        Err(StoreError::Unavailable)
    ));
}
