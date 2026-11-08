use croaker_protocol::{enrollment::*, *};
use croaker_relay::{identity::*, *};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::sync::Arc;
fn binding(tenant: TenantId, device: DeviceId, key: &SigningKey) -> IdentityBinding {
    IdentityBinding {
        tenant_id: tenant,
        device_id: device,
        signature_key: key.verifying_key().to_bytes().to_vec(),
    }
}
async fn contract(store: Arc<dyn Store>) {
    let tenant = TenantId::new();
    let old = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Member,
    };
    let admin = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Admin,
    };
    let outsider = Principal {
        tenant_id: TenantId::new(),
        device_id: DeviceId::new(),
        role: Role::Admin,
    };
    let colleague = Principal {
        tenant_id: tenant,
        device_id: DeviceId::new(),
        role: Role::Member,
    };
    for (actor, seed) in [
        (&old, "a"),
        (&admin, "b"),
        (&outsider, "c"),
        (&colleague, "e"),
    ] {
        store.provision(actor, &seed.repeat(64)).await.unwrap();
    }
    let old_key = SigningKey::from_bytes(&rand::random());
    let grant = AccountGrant {
        issuer: "https://id.example".into(),
        subject: MessageId::new().to_string(),
        tenant_id: tenant,
        role: Role::Member,
        name: "Test Member".into(),
        bootstrap_device: Some(binding(tenant, old.device_id, &old_key)),
    };
    let account = grant.id();
    store.grant_account(grant.clone()).await.unwrap();
    store.grant_account(grant.clone()).await.unwrap();
    let candidate = SigningKey::from_bytes(&rand::random());
    let device = DeviceId::new();
    let token = "d".repeat(64);
    let mut input = EnrollmentInput {
        id: MessageId::new(),
        binding: binding(tenant, device, &candidate),
        label: "Replacement".into(),
        token_hash: hex::encode(Sha256::digest(token.as_bytes())),
        proof: vec![],
    };
    input.proof = candidate
        .sign(&input.signing_bytes(&account))
        .to_bytes()
        .to_vec();
    let mut bad = input.clone();
    bad.binding.device_id = DeviceId::new();
    assert!(store.request_enrollment(&account, bad).await.is_err());
    store
        .request_enrollment(&account, input.clone())
        .await
        .unwrap();
    store
        .request_enrollment(&account, input.clone())
        .await
        .unwrap();
    assert!(store.authenticate(&token).await.is_err());
    let fingerprint = input.binding.fingerprint();
    for actor in [&outsider, &colleague] {
        assert!(
            store
                .decide_enrollment(actor, &account, input.id, &fingerprint, true)
                .await
                .is_err()
        );
    }
    assert!(
        store
            .decide_enrollment(&old, &account, input.id, &"0".repeat(64), true)
            .await
            .is_err()
    );
    store
        .decide_enrollment(&old, &account, input.id, &fingerprint, true)
        .await
        .unwrap();
    store
        .decide_enrollment(&old, &account, input.id, &fingerprint, true)
        .await
        .unwrap();
    let approved = store.authenticate(&token).await.unwrap();
    assert_eq!(approved.device_id, device);
    assert_eq!(approved.role, Role::Member);
    assert!(
        !store
            .account(&account)
            .await
            .unwrap()
            .public()
            .to_string()
            .contains(&input.token_hash)
    );
    // Credential collision must roll back BOTH approval and device registration.
    let second_key = SigningKey::from_bytes(&rand::random());
    let mut collision = EnrollmentInput {
        id: MessageId::new(),
        binding: binding(tenant, DeviceId::new(), &second_key),
        label: "Collision fixture".into(),
        token_hash: input.token_hash.clone(),
        proof: vec![],
    };
    collision.proof = second_key
        .sign(&collision.signing_bytes(&account))
        .to_bytes()
        .to_vec();
    store
        .request_enrollment(&account, collision.clone())
        .await
        .unwrap();
    assert!(
        store
            .decide_enrollment(
                &old,
                &account,
                collision.id,
                &collision.binding.fingerprint(),
                true
            )
            .await
            .is_err()
    );
    let unchanged = store.account(&account).await.unwrap();
    assert!(
        !unchanged
            .devices
            .iter()
            .any(|d| d.binding.device_id == collision.binding.device_id)
    );
    assert_eq!(
        unchanged
            .requests
            .iter()
            .find(|r| r.input.id == collision.id)
            .unwrap()
            .status,
        EnrollmentStatus::Pending
    );
    assert_eq!(store.authenticate(&token).await.unwrap().device_id, device);
    // An approved device still has no membership in the old conversation.
    let group = store
        .create(
            &admin,
            CreateConversation {
                id: ConversationId::new(),
                devices: vec![admin.device_id, old.device_id],
            },
        )
        .await
        .unwrap();
    assert!(store.sync(&approved, group.id, 0, 32).await.is_err());
    // The replacement may retire its own account's old device, never a colleague.
    assert!(
        store
            .revoke_owned_device(&approved, &account, colleague.device_id)
            .await
            .is_err()
    );
    store
        .revoke_owned_device(&approved, &account, old.device_id)
        .await
        .unwrap();
    assert!(store.authenticate(&"a".repeat(64)).await.is_err());
    store.grant_account(grant).await.unwrap(); // Restart/bootstrap must tolerate a retired initial device.
    assert!(
        store
            .decide_enrollment(&old, &account, input.id, &fingerprint, true)
            .await
            .is_err()
    );
    store
        .revoke_owned_device(&approved, &account, device)
        .await
        .unwrap();
    assert!(store.authenticate(&token).await.is_err());
}
#[tokio::test]
async fn sqlite_enrollment_contract() {
    contract(Arc::new(sqlite::SqliteStore::open(":memory:").unwrap())).await;
}
#[cfg(feature = "aws")]
#[tokio::test]
#[ignore = "requires DynamoDB Local; run scripts/test-dynamodb.sh"]
async fn dynamodb_enrollment_contract() {
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
    let table = format!("croaker-enrollment-test-{}", MessageId::new());
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
    contract(Arc::new(dynamodb::DynamoStore::new(
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
