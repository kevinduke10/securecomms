use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_client::{ApprovedAddition, ClientStore};
use croaker_crypto::Device;
use croaker_protocol::*;
use zeroize::Zeroizing;

fn store(device: Device, key: u8) -> ClientStore {
    ClientStore::create(":memory:", device, Zeroizing::new([key; 32])).unwrap()
}
fn page(sender: DeviceId, event: SubmitEvent, sequence: u64) -> SyncPage {
    SyncPage {
        events: vec![StoredEvent {
            sequence,
            sender,
            submitted_at: 1_800_000_000,
            expires_at: 1_900_000_000,
            event,
        }],
        next_cursor: sequence,
        has_more: false,
    }
}
fn sorted(devices: &[DeviceId]) -> Vec<DeviceId> {
    let mut devices = devices.to_vec();
    devices.sort();
    devices
}
fn group() -> (ConversationId, Device, Device, Device, Device) {
    let tenant = TenantId::new();
    let id = ConversationId::new();
    let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
    let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
    let carol = Device::new(tenant, DeviceId::new()).unwrap();
    let dave = Device::new(tenant, DeviceId::new()).unwrap();
    alice.create_conversation(id).unwrap();
    let welcome = alice
        .add_member(id, &bob.key_package().unwrap(), bob.binding())
        .unwrap();
    bob.join(id, &welcome.welcome, std::slice::from_ref(alice.binding()))
        .unwrap();
    (id, alice, bob, carol, dave)
}
#[test]
fn saved_admission_retries_after_restart_and_requires_the_exact_pinned_addition() {
    let (id, alice, bob, mut carol, _) = group();
    let authority = alice.binding().clone();
    let bob_pin = bob.binding().clone();
    let added = carol.binding().clone();
    let package = carol.key_package().unwrap();
    let expected = sorted(&[authority.device_id, bob_pin.device_id]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owner.sqlite");
    let mut alice = ClientStore::create(&path, alice, Zeroizing::new([11; 32])).unwrap();
    let request = MessageId::new();
    let admission = alice
        .queue_admission(id, request, &expected, &package, &added)
        .unwrap();
    drop(alice);
    let mut alice = ClientStore::open(&path, Zeroizing::new([11; 32])).unwrap();
    assert_eq!(alice.pending_admissions().unwrap()[0].input, admission);
    assert_eq!(
        alice
            .queue_admission(id, request, &expected, &package, &added)
            .unwrap(),
        admission
    );
    alice.acknowledge_admission(request).unwrap();
    assert!(alice.pending_admissions().unwrap().is_empty());
    assert_eq!(
        alice
            .queue_admission(id, request, &expected, &package, &added)
            .unwrap(),
        admission
    );
    let mut changed = added.clone();
    changed.signature_key[0] ^= 1;
    assert!(
        alice
            .queue_admission(id, request, &expected, &package, &changed)
            .is_err()
    );
    let mut bob = store(bob, 12);
    let mut bad = ApprovedAddition {
        commit_id: request,
        authority: authority.clone(),
        added: changed,
    };
    let commit = page(authority.device_id, admission.commit.clone(), 1);
    assert!(
        bob.apply_page_with_membership(id, 0, &commit, &[], |_| false, std::slice::from_ref(&bad))
            .is_err()
    );
    assert_eq!(bob.cursor(id).unwrap(), 0);
    bad.added = added.clone();
    bob.apply_page_with_membership(id, 0, &commit, &[], |_| false, &[bad])
        .unwrap();
    let mut carol = store(carol, 13);
    carol.prepare_join(id, 1).unwrap();
    carol.prepare_join(id, 1).unwrap();
    carol
        .apply_page(
            id,
            1,
            &page(authority.device_id, admission.welcome, 2),
            &[authority.clone(), bob_pin],
        )
        .unwrap();
    assert!(carol.history(id).unwrap().is_empty());
    let message = alice
        .queue_message(id, b"Future message after approved admission")
        .unwrap();
    let delivered = page(authority.device_id, message.event, 3);
    carol.apply_page(id, 2, &delivered, &[]).unwrap();
    bob.apply_page(id, 1, &delivered, &[]).unwrap();
    assert_eq!(
        carol.history(id).unwrap()[0].plaintext,
        b"Future message after approved admission"
    );
    assert!(carol.prepare_join(id, 1).is_err());
}
#[test]
fn delayed_saved_envelopes_decrypt_one_epoch_then_are_reported_stale_after_two_additions() {
    let (id, alice, bob, mut carol, mut dave) = group();
    let a = alice.binding().clone();
    let b = bob.binding().clone();
    let c = carol.binding().clone();
    let d = dave.binding().clone();
    let owner_directory = tempfile::tempdir().unwrap();
    let owner_path = owner_directory.path().join("owner.sqlite");
    let mut alice = ClientStore::create(&owner_path, alice, Zeroizing::new([21; 32])).unwrap();
    let mut bob = store(bob, 22);
    let old_soon = bob
        .queue_message(id, b"Saved envelope delivered after one admission")
        .unwrap();
    let old_late = bob
        .queue_message(id, b"Saved envelope delivered after two admissions")
        .unwrap();
    let package = carol.key_package().unwrap();
    let first = alice
        .queue_admission(
            id,
            MessageId::new(),
            &sorted(&[a.device_id, b.device_id]),
            &package,
            &c,
        )
        .unwrap();
    alice.acknowledge_admission(first.request_id).unwrap();
    let approval1 = ApprovedAddition {
        commit_id: first.commit.message_id,
        authority: a.clone(),
        added: c.clone(),
    };
    bob.apply_page_with_membership(
        id,
        0,
        &page(a.device_id, first.commit, 1),
        &[],
        |_| false,
        &[approval1],
    )
    .unwrap();
    let mut carol = store(carol, 23);
    carol.prepare_join(id, 1).unwrap();
    carol
        .apply_page(
            id,
            1,
            &page(a.device_id, first.welcome, 2),
            &[a.clone(), b.clone()],
        )
        .unwrap();
    // Sender's exact saved old ciphertext is still decryptable by existing members.
    alice
        .apply_page(id, 0, &page(b.device_id, old_soon.event.clone(), 3), &[])
        .unwrap();
    assert_eq!(
        alice.history(id).unwrap()[0].plaintext,
        b"Saved envelope delivered after one admission"
    );
    let report = carol
        .apply_page_with_membership_report(
            id,
            2,
            &page(b.device_id, old_soon.event.clone(), 3),
            &[],
            |_| false,
            &[],
        )
        .unwrap();
    assert_eq!(report.skipped_stale_applications, 1);
    assert!(carol.history(id).unwrap().is_empty());
    let package = dave.key_package().unwrap();
    let second = alice
        .queue_admission(
            id,
            MessageId::new(),
            &sorted(&[a.device_id, b.device_id, c.device_id]),
            &package,
            &d,
        )
        .unwrap();
    alice.acknowledge_admission(second.request_id).unwrap();
    let approval2 = ApprovedAddition {
        commit_id: second.commit.message_id,
        authority: a.clone(),
        added: d.clone(),
    };
    bob.apply_page_with_membership(
        id,
        1,
        &page(a.device_id, second.commit.clone(), 4),
        &[],
        |_| false,
        std::slice::from_ref(&approval2),
    )
    .unwrap();
    carol
        .apply_page_with_membership(
            id,
            3,
            &page(a.device_id, second.commit, 4),
            &[],
            |_| false,
            &[approval2],
        )
        .unwrap();
    let mut dave = store(dave, 24);
    dave.prepare_join(id, 4).unwrap();
    dave.apply_page(
        id,
        4,
        &page(a.device_id, second.welcome, 5),
        &[a.clone(), b.clone(), c.clone()],
    )
    .unwrap();
    let current = alice
        .queue_message(id, b"Current epoch remains usable")
        .unwrap();
    let mut corrupted = current.event.clone();
    let mut bytes = BASE64_STANDARD.decode(&corrupted.ciphertext).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    corrupted.ciphertext = BASE64_STANDARD.encode(bytes);
    let stale = page(b.device_id, old_late.event.clone(), 6);
    let mut mixed = stale.clone();
    mixed
        .events
        .push(page(a.device_id, corrupted, 7).events.remove(0));
    mixed.next_cursor = 7;
    assert!(
        dave.apply_page_with_membership_report(id, 5, &mixed, &[], |_| false, &[])
            .is_err()
    );
    assert_eq!(dave.cursor(id).unwrap(), 5);
    assert_eq!(dave.stale_application_count(id).unwrap(), 0);
    for client in [&mut alice, &mut carol, &mut dave] {
        let cursor = client.cursor(id).unwrap();
        let report = client
            .apply_page_with_membership_report(id, cursor, &stale, &[], |_| false, &[])
            .unwrap();
        assert_eq!(report.skipped_stale_applications, 1);
        assert!(report.transient.is_empty());
        assert!(client.stale_application_count(id).unwrap() >= 1);
    }
    // Retrying was never re-encryption: sender still holds both original envelopes.
    assert_eq!(bob.pending().unwrap(), vec![old_soon, old_late.clone()]);
    let valid = page(a.device_id, current.event, 7);
    for client in [&mut bob, &mut carol, &mut dave] {
        let cursor = client.cursor(id).unwrap();
        client.apply_page(id, cursor, &valid, &[]).unwrap();
        assert!(
            client
                .history(id)
                .unwrap()
                .iter()
                .any(|m| m.plaintext == b"Current epoch remains usable")
        );
    }
    let mut foreign = Device::new(TenantId::new(), DeviceId::new()).unwrap();
    foreign.create_conversation(id).unwrap();
    let ciphertext = foreign.encrypt(id, b"wrong group").unwrap();
    let bad = SubmitEvent {
        protocol_version: 1,
        message_id: MessageId::new(),
        kind: EventKind::Application,
        recipient: None,
        ciphertext: BASE64_STANDARD.encode(ciphertext),
    };
    assert!(
        dave.apply_page(id, 7, &page(b.device_id, bad, 8), &[])
            .is_err()
    );
    assert_eq!(dave.cursor(id).unwrap(), 7);
    drop(alice);
    let alice = ClientStore::open(&owner_path, Zeroizing::new([21; 32])).unwrap();
    assert_eq!(alice.stale_application_count(id).unwrap(), 1);
    assert_eq!(alice.cursor(id).unwrap(), 6);
}
