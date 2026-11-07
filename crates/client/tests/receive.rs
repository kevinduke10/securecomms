use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_client::ClientStore;
use croaker_crypto::Device;
use croaker_protocol::*;
use zeroize::Zeroizing;

#[test]
fn invalid_receive_page_rolls_back_history_cursor_and_ratchet_together() {
    let tenant = TenantId::new();
    let conversation = ConversationId::new();
    let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
    let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
    alice.create_conversation(conversation).unwrap();
    let added = alice
        .add_member(conversation, &bob.key_package().unwrap(), bob.binding())
        .unwrap();
    bob.join(conversation, &added.welcome, &[alice.binding().clone()])
        .unwrap();
    let mut store = ClientStore::create(":memory:", bob, Zeroizing::new([61; 32])).unwrap();
    let mut events = Vec::new();
    for (index, plaintext) in [b"first message".as_slice(), b"second message".as_slice()]
        .iter()
        .enumerate()
    {
        events.push(StoredEvent {
            sequence: index as u64 + 1,
            sender: alice.binding().device_id,
            submitted_at: 100,
            expires_at: u64::MAX,
            event: SubmitEvent {
                protocol_version: API_VERSION,
                message_id: MessageId::new(),
                kind: EventKind::Application,
                recipient: None,
                ciphertext: BASE64_STANDARD.encode(alice.encrypt(conversation, plaintext).unwrap()),
            },
        });
    }
    let good = SyncPage {
        events,
        next_cursor: 2,
        has_more: false,
    };
    let mut bad = SyncPage {
        events: good.events.clone(),
        next_cursor: 2,
        has_more: false,
    };
    let mut bytes = BASE64_STANDARD
        .decode(&bad.events[1].event.ciphertext)
        .unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    bad.events[1].event.ciphertext = BASE64_STANDARD.encode(bytes);
    assert!(store.apply_page(conversation, 0, &bad, &[]).is_err());
    assert_eq!(store.cursor(conversation).unwrap(), 0);
    assert!(store.history(conversation).unwrap().is_empty());
    store.apply_page(conversation, 0, &good, &[]).unwrap();
    assert_eq!(store.cursor(conversation).unwrap(), 2);
    assert_eq!(store.history(conversation).unwrap().len(), 2);
    assert_eq!(
        store.history(conversation).unwrap()[1].plaintext,
        b"second message"
    );
    assert!(store.apply_page(conversation, 0, &good, &[]).is_err());
}

#[test]
fn transient_signals_advance_ratchets_without_entering_either_history() {
    let tenant = TenantId::new();
    let conversation = ConversationId::new();
    let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
    let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
    alice.create_conversation(conversation).unwrap();
    let added = alice
        .add_member(conversation, &bob.key_package().unwrap(), bob.binding())
        .unwrap();
    bob.join(conversation, &added.welcome, &[alice.binding().clone()])
        .unwrap();
    let sender = alice.binding().device_id;
    let mut alice = ClientStore::create(":memory:", alice, Zeroizing::new([31; 32])).unwrap();
    let mut bob = ClientStore::create(":memory:", bob, Zeroizing::new([32; 32])).unwrap();
    let pulse = alice
        .queue_transient(conversation, b"typing-pulse")
        .unwrap();
    let message = alice
        .queue_message(conversation, b"actual message")
        .unwrap();
    assert_eq!(alice.history(conversation).unwrap().len(), 1);
    let events = [pulse, message]
        .into_iter()
        .enumerate()
        .map(|(index, event)| StoredEvent {
            sequence: index as u64 + 1,
            sender,
            submitted_at: 100,
            expires_at: u64::MAX,
            event: event.event,
        })
        .collect();
    let page = SyncPage {
        events,
        next_cursor: 2,
        has_more: false,
    };
    let signals = bob
        .apply_page_with_transient(conversation, 0, &page, &[], |bytes| {
            bytes == b"typing-pulse"
        })
        .unwrap();
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].sequence, 1);
    assert_eq!(signals[0].message.plaintext, b"typing-pulse");
    assert_eq!(bob.cursor(conversation).unwrap(), 2);
    assert_eq!(bob.history(conversation).unwrap().len(), 1);
    assert_eq!(
        bob.history(conversation).unwrap()[0].plaintext,
        b"actual message"
    );
}
