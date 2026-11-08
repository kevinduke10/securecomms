use croaker_client::ClientStore;
use croaker_crypto::Device;
use croaker_protocol::*;
use zeroize::Zeroizing;
#[test]
fn durable_media_retry_reuses_key_and_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client");
    let key = [9; 32];
    let d = Device::new(TenantId::new(), DeviceId::new()).unwrap();
    let mut client = ClientStore::create(&path, d, Zeroizing::new(key)).unwrap();
    let c = ConversationId::new();
    let request = MessageId::new();
    let bytes = vec![123; 65536];
    let first = client
        .prepare_media(request, c, "secret-name.pdf", "application/pdf", &bytes)
        .unwrap();
    drop(client);
    let mut client = ClientStore::open(&path, Zeroizing::new(key)).unwrap();
    let again = client
        .prepare_media(request, c, "secret-name.pdf", "application/pdf", &bytes)
        .unwrap();
    assert!(first.descriptor == again.descriptor);
    assert_eq!(first.ciphertext, again.ciphertext);
    assert!(
        client
            .prepare_media(request, c, "changed.pdf", "application/pdf", &bytes)
            .is_err()
    );
    let db = std::fs::read(path).unwrap();
    assert!(!db.windows(15).any(|w| w == b"secret-name.pdf"));
    assert!(!db.windows(44).any(|w| w == first.descriptor.key.as_bytes()));
}
#[test]
fn acknowledged_upload_compacts_without_regenerating_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client");
    let key = [5; 32];
    let d = Device::new(TenantId::new(), DeviceId::new()).unwrap();
    let mut client = ClientStore::create(&path, d, Zeroizing::new(key)).unwrap();
    let c = ConversationId::new();
    let id = MessageId::new();
    let bytes = vec![81; 65536];
    let original = client.prepare_media(id, c, "file.bin", "", &bytes).unwrap();
    client.mark_media_uploaded(id).unwrap();
    client.mark_media_uploaded(id).unwrap();
    drop(client);
    let mut client = ClientStore::open(&path, Zeroizing::new(key)).unwrap();
    let retry = client.prepare_media(id, c, "file.bin", "", &bytes).unwrap();
    assert!(original.descriptor == retry.descriptor);
    assert!(retry.ciphertext.is_empty());
    assert!(
        client
            .prepare_media(id, c, "file.bin", "", b"different")
            .is_err()
    );
    assert!(client.mark_media_uploaded(MessageId::new()).is_err());
}
