use super::*;
use croaker_relay::{Store, sqlite::SqliteStore};
use std::sync::Mutex;
#[derive(Default)]
struct MemorySecrets(Mutex<BTreeMap<String, Vec<u8>>>);
impl SecretStore for MemorySecrets {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .map(Zeroizing::new))
    }
    fn set(&self, name: &str, value: &[u8]) -> Result<()> {
        self.0.lock().unwrap().insert(name.into(), value.to_vec());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<()> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}
fn fixture(
    directory: PathBuf,
    secrets: Arc<MemorySecrets>,
    tenant: TenantId,
    relay: &str,
    name: &str,
) -> DeviceRuntime {
    let mut runtime = DeviceRuntime::open(directory, secrets).unwrap();
    runtime.connection = Some(Connection {
        relay: relay.into(),
    });
    atomic_write(
        &runtime.directory.join("connection.json"),
        &serde_json::to_vec(&runtime.connection).unwrap(),
    )
    .unwrap();
    let device = Device::new(tenant, DeviceId::new()).unwrap();
    let binding = device.binding().clone();
    let token = random_token();
    runtime.client = Some(
        ClientStore::create(
            runtime.directory.join("client.sqlite"),
            device,
            Zeroizing::new(*runtime.key),
        )
        .unwrap(),
    );
    runtime.private = Some(Private {
        relay: relay.into(),
        binding: binding.clone(),
        token: token.clone(),
        account_id: hex::encode(Sha256::digest(name)),
        name: name.into(),
        role: Role::Member,
        enrollment: EnrollmentInput {
            id: MessageId::new(),
            binding: IdentityBinding {
                tenant_id: tenant,
                device_id: binding.device_id,
                signature_key: binding.signature_key.clone(),
            },
            label: name.into(),
            token_hash: hex::encode(Sha256::digest(token.as_bytes())),
            proof: vec![],
        },
        approved: true,
        locked: false,
        contacts: BTreeMap::new(),
        approvals: vec![],
        lifecycles: vec![],
        group_decisions: vec![],
        chats: BTreeMap::new(),
        read_receipts: false,
    });
    runtime.save().unwrap();
    runtime
}
async fn ok(runtime: &mut DeviceRuntime, method: &str, path: &str, body: Option<Value>) -> Value {
    let response = runtime.request(method, path, body).await;
    assert_eq!(response.status, 200, "{}", response.body);
    response.body
}
#[test]
fn exclusive_store_and_missing_key_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let runtime = DeviceRuntime::open(dir.path().into(), secrets).unwrap();
    assert!(DeviceRuntime::open(dir.path().into(), Arc::new(MemorySecrets::default())).is_err());
    std::fs::write(dir.path().join("device.vault"), b"existing ciphertext").unwrap();
    drop(runtime);
    assert!(DeviceRuntime::open(dir.path().into(), Arc::new(MemorySecrets::default())).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("device.vault")).unwrap(),
        b"existing ciphertext"
    );
}
#[tokio::test]
async fn independent_stores_verified_groups_offline_retry_and_restart() {
    let relay = Arc::new(SqliteStore::open(":memory:").unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let media_dir = tempfile::tempdir().unwrap();
    let media = Arc::new(croaker_relay::media::ObjectService {
        relay: relay.clone(),
        registry: Arc::new(
            croaker_relay::media::local::SqliteRegistry::open(
                media_dir.path().join("media.sqlite"),
            )
            .unwrap(),
        ),
        objects: Arc::new(
            croaker_relay::media::local::Files::open(media_dir.path().join("objects"), url.clone())
                .unwrap(),
        ),
    });
    let router = croaker_relay::api::router(relay.clone())
        .merge(croaker_relay::media::router(media.clone()))
        .merge(croaker_relay::media::local_content_router(media));
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let tenant = TenantId::new();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let ak = Arc::new(MemorySecrets::default());
    let bk = Arc::new(MemorySecrets::default());
    let mut alice = fixture(
        a.path().into(),
        ak.clone(),
        tenant,
        &url,
        "Alice native fixture",
    );
    let mut bob = fixture(
        b.path().into(),
        bk.clone(),
        tenant,
        &url,
        "Bob native fixture",
    );
    for runtime in [&alice, &bob] {
        let p = runtime.profile().unwrap();
        relay
            .provision(
                &Principal {
                    tenant_id: tenant,
                    device_id: p.binding.device_id,
                    role: p.role,
                },
                &p.token,
            )
            .await
            .unwrap();
    }
    let ai = alice.profile().unwrap().binding.device_id;
    let bi = bob.profile().unwrap().binding.device_id;
    let ac = ok(&mut alice, "GET", "/desktop/contact", None).await;
    let bc = ok(&mut bob, "GET", "/desktop/contact", None).await;
    assert_eq!(
        alice
            .request(
                "POST",
                "/desktop/contacts",
                Some(json!({"contact":bc,"fingerprint":"wrong"}))
            )
            .await
            .status,
        400
    );
    let fingerprint = bob.profile().unwrap().binding.fingerprint();
    ok(
        &mut alice,
        "POST",
        "/desktop/contacts",
        Some(json!({"contact":bc,"fingerprint":fingerprint})),
    )
    .await;
    let fingerprint = alice.profile().unwrap().binding.fingerprint();
    ok(
        &mut bob,
        "POST",
        "/desktop/contacts",
        Some(json!({"contact":ac,"fingerprint":fingerprint})),
    )
    .await;
    let chat = ok(
        &mut alice,
        "POST",
        &format!("/local-api/{ai}/conversations"),
        Some(json!({"title":"Independent devices","kind":"group","members":[bi]})),
    )
    .await;
    let id: ConversationId = serde_json::from_value(chat["id"].clone()).unwrap();
    let invites = ok(&mut alice, "GET", "/desktop/invitations", None).await;
    ok(
        &mut bob,
        "POST",
        "/desktop/invitations",
        Some(json!({"invitation":invites[0],"fingerprint":fingerprint})),
    )
    .await;
    ok(
        &mut bob,
        "GET",
        &format!("/local-api/state?profile={bi}"),
        None,
    )
    .await;
    ok(
        &mut alice,
        "POST",
        &format!("/local-api/{ai}/online"),
        Some(json!({"online":false})),
    )
    .await;
    let request = MessageId::new();
    let content = json!({"version":1,"text":"Private fixture content not stored in plaintext","attachment":null});
    let body = json!({"conversation_id":id,"request_id":request,"content":content});
    ok(
        &mut alice,
        "POST",
        &format!("/local-api/{ai}/send"),
        Some(body.clone()),
    )
    .await;
    let pending = alice.client().unwrap().pending().unwrap();
    assert_eq!(pending.len(), 1);
    drop(alice);
    let mut alice = DeviceRuntime::open(a.path().into(), ak).unwrap();
    assert_eq!(alice.client().unwrap().pending().unwrap(), pending);
    ok(
        &mut alice,
        "POST",
        &format!("/local-api/{ai}/send"),
        Some(body),
    )
    .await;
    let received = ok(
        &mut bob,
        "GET",
        &format!("/local-api/state?profile={bi}"),
        None,
    )
    .await;
    assert_eq!(
        received["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        received["conversations"][0]["messages"][0]["text"],
        content["text"]
    );
    let acknowledged = ok(
        &mut alice,
        "GET",
        &format!("/local-api/state?profile={ai}"),
        None,
    )
    .await;
    assert_eq!(
        acknowledged["conversations"][0]["messages"][0]["receipts"]["delivered"],
        json!([bi])
    );
    // Objects cross the HTTP relay as ciphertext; only summary + verified plaintext
    // leave the native engine. Repeating an acknowledged upload keeps its object ID.
    let file = vec![73; 64 * 1024];
    let media_request = MessageId::new();
    let media_body = json!({"conversation_id":id,"request_id":media_request,"name":"native-file.bin","mime":"application/octet-stream","data":BASE64_STANDARD.encode(&file),"text":"","reply_to":null});
    ok(
        &mut alice,
        "POST",
        &format!("/local-api/{ai}/media/send"),
        Some(media_body.clone()),
    )
    .await;
    ok(
        &mut alice,
        "POST",
        &format!("/local-api/{ai}/media/send"),
        Some(media_body),
    )
    .await;
    let state = ok(
        &mut bob,
        "GET",
        &format!("/local-api/state?profile={bi}"),
        None,
    )
    .await;
    let messages = state["conversations"][0]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    let media = &messages[1]["media"];
    assert!(media["key"].is_null());
    assert!(media["sha256"].is_null());
    let downloaded = ok(
        &mut bob,
        "GET",
        &format!("/local-api/{bi}/media/{id}/{media_request}?sender={ai}"),
        None,
    )
    .await;
    assert_eq!(
        BASE64_STANDARD
            .decode(downloaded["binary_base64"].as_str().unwrap())
            .unwrap(),
        file
    );
    // The same native group management API validates a transferred owner's exact
    // decision packet before applying its authenticated MLS commit on the peer.
    let request = MessageId::new();
    let fingerprint = bob.profile().unwrap().binding.fingerprint();
    ok(&mut alice,"POST",&format!("/local-api/{ai}/conversations/{id}/lifecycle"),Some(json!({"request_id":request,"target_profile":bi,"fingerprint":fingerprint,"action":"transfer"}))).await;
    let decisions = ok(&mut alice, "GET", "/desktop/group-decisions", None).await;
    let fingerprint = alice.profile().unwrap().binding.fingerprint();
    ok(
        &mut bob,
        "POST",
        "/desktop/group-decisions",
        Some(json!({"decision":decisions[0],"fingerprint":fingerprint})),
    )
    .await;
    let state = ok(
        &mut bob,
        "GET",
        &format!("/local-api/state?profile={bi}"),
        None,
    )
    .await;
    assert_eq!(state["sync_error"], Value::Null);
    assert_eq!(state["conversations"][0]["owner"], json!(bi));
    assert_eq!(
        alice
            .request("GET", "https://attacker.invalid", None)
            .await
            .status,
        400
    );
    assert_eq!(
        alice
            .request("GET", &format!("/local-api/state?profile={bi}"), None)
            .await
            .status,
        400
    );
    for directory in [a.path(), b.path()] {
        for file in std::fs::read_dir(directory).unwrap() {
            let path = file.unwrap().path();
            if path.is_file() {
                let bytes = std::fs::read(path).unwrap();
                assert!(
                    !bytes
                        .windows(b"Private fixture content".len())
                        .any(|w| w == b"Private fixture content")
                );
                assert!(!bytes.windows(20).any(|w| w == b"Alice native fixture"));
            }
        }
    }
    server.abort();
}

#[test]
fn relay_configuration_tampering_and_failed_profile_save_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let mut runtime = fixture(
        directory.path().into(),
        secrets.clone(),
        TenantId::new(),
        "https://original.example",
        "Native fixture",
    );
    // A failed profile save cannot leave a publishable in-memory-only identity.
    std::fs::create_dir(directory.path().join("device.pending")).unwrap();
    runtime.private.as_mut().unwrap().name = "Unsaved edit".into();
    assert!(runtime.save().is_err());
    assert!(runtime.ensure_healthy().is_err());
    drop(runtime);
    std::fs::remove_dir(directory.path().join("device.pending")).unwrap();
    atomic_write(
        &directory.path().join("connection.json"),
        br#"{"relay":"https://changed.example"}"#,
    )
    .unwrap();
    assert!(DeviceRuntime::open(directory.path().into(), secrets.clone()).is_err());
    atomic_write(
        &directory.path().join("connection.json"),
        br#"{"relay":"https://original.example"}"#,
    )
    .unwrap();
    let runtime = DeviceRuntime::open(directory.path().into(), secrets).unwrap();
    assert_eq!(runtime.profile().unwrap().name, "Native fixture");
}

#[tokio::test]
async fn explicit_sign_out_remains_locked_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let mut runtime = fixture(
        directory.path().into(),
        secrets.clone(),
        TenantId::new(),
        "https://native-fixture.example",
        "Signed-out fixture",
    );
    let device = runtime.profile().unwrap().binding.device_id;
    ok(&mut runtime, "POST", "/local-auth/logout", Some(json!({}))).await;
    drop(runtime);
    let mut reopened = DeviceRuntime::open(directory.path().into(), secrets).unwrap();
    let session = ok(&mut reopened, "GET", "/local-auth/me", None).await;
    assert_eq!(session["authenticated"], false);
    let response = reopened
        .request("GET", &format!("/local-api/state?profile={device}"), None)
        .await;
    assert_eq!(response.status, 400);
    assert!(response.body["error"].as_str().unwrap().contains("Sign in"));
    assert!(
        reopened.profile().unwrap().approved,
        "Sign-out locks history, preserving the approved device identity."
    );
}
