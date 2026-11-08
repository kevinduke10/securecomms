use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_client::{ClientStore, ReceivePolicy, WelcomeAuthorization};
use croaker_crypto::Device;
use croaker_protocol::*;
use zeroize::Zeroizing;
fn page(sender: DeviceId, welcome: Vec<u8>) -> SyncPage {
    SyncPage {
        events: vec![StoredEvent {
            sequence: 1,
            sender,
            event: SubmitEvent {
                protocol_version: API_VERSION,
                message_id: MessageId::new(),
                kind: EventKind::Welcome,
                recipient: None,
                ciphertext: BASE64_STANDARD.encode(welcome),
            },
            submitted_at: 1,
            expires_at: u64::MAX,
        }],
        next_cursor: 1,
        has_more: false,
    }
}
#[test]
fn pinned_nonowner_welcome_and_wrong_roster_roll_back_before_authorized_join() {
    let tenant = TenantId::new();
    let id = ConversationId::new();
    let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
    let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
    let mut carol = Device::new(tenant, DeviceId::new()).unwrap();
    let authority = alice.binding().clone();
    let bob_pin = bob.binding().clone();
    let carol_pin = carol.binding().clone();
    let peers = vec![authority.clone(), bob_pin.clone()];
    let expected = vec![authority.device_id, bob_pin.device_id, carol_pin.device_id];
    let malicious_package = carol.key_package().unwrap();
    let authorized_package = carol.key_package().unwrap();
    // All three identities are pinned. Bob fabricates the same conversation ID
    // and exact roster, but is not the locally authorized conversation owner.
    bob.create_conversation(id).unwrap();
    let alice_package = alice.key_package().unwrap();
    let forged = bob
        .add_members(
            id,
            &[
                (&alice_package, &authority),
                (&malicious_package, &carol_pin),
            ],
        )
        .unwrap();
    let mut recipient = ClientStore::create(":memory:", carol, Zeroizing::new([3; 32])).unwrap();
    let forged = page(authority.device_id, forged.welcome);
    let apply = |store: &mut ClientStore, page: &SyncPage, members: &[DeviceId]| {
        store.apply_page_with_policy_report(
            id,
            0,
            page,
            &peers,
            |_| false,
            ReceivePolicy {
                welcome: Some(WelcomeAuthorization {
                    authority: &authority,
                    members,
                }),
                ..ReceivePolicy::default()
            },
        )
    };
    assert!(apply(&mut recipient, &forged, &expected).is_err());
    assert_eq!(recipient.cursor(id).unwrap(), 0);
    assert!(!recipient.update(|d| Ok(d.has_conversation(id))).unwrap());
    assert!(recipient.history(id).unwrap().is_empty());
    alice.create_conversation(id).unwrap();
    let bob_package = bob.key_package().unwrap();
    let legitimate = alice
        .add_members(
            id,
            &[(&bob_package, &bob_pin), (&authorized_package, &carol_pin)],
        )
        .unwrap();
    let legitimate = page(authority.device_id, legitimate.welcome);
    let missing_bob = vec![authority.device_id, carol_pin.device_id];
    assert!(apply(&mut recipient, &legitimate, &missing_bob).is_err());
    assert_eq!(recipient.cursor(id).unwrap(), 0);
    assert!(!recipient.update(|d| Ok(d.has_conversation(id))).unwrap());
    let duplicate = vec![
        authority.device_id,
        bob_pin.device_id,
        carol_pin.device_id,
        bob_pin.device_id,
    ];
    assert!(apply(&mut recipient, &legitimate, &duplicate).is_err());
    assert_eq!(recipient.cursor(id).unwrap(), 0);
    apply(&mut recipient, &legitimate, &expected).unwrap();
    assert_eq!(recipient.cursor(id).unwrap(), 1);
    let mut actual = recipient.update(|d| d.conversation_devices(id)).unwrap();
    actual.sort();
    let mut expected = expected;
    expected.sort();
    assert_eq!(actual, expected);
}
