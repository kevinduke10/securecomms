use croaker_client::{ClientStore, lifecycle::ApprovedLifecycle};
use croaker_crypto::Device;
use croaker_protocol::{lifecycle::*, *};
use zeroize::Zeroizing;
#[test]
fn durable_transfer_retry_and_recipient_rollback() {
    let tenant = TenantId::new();
    let id = ConversationId::new();
    let mut a = Device::new(tenant, DeviceId::new()).unwrap();
    let mut b = Device::new(tenant, DeviceId::new()).unwrap();
    a.create_conversation(id).unwrap();
    let added = a
        .add_member(id, &b.key_package().unwrap(), b.binding())
        .unwrap();
    b.join(id, &added.welcome, &[a.binding().clone()]).unwrap();
    let authority = a.binding().clone();
    let target = b.binding().clone();
    let context = LifecycleContext {
        conversation_id: id,
        request_id: MessageId::new(),
        expected_owner: authority.device_id,
        expected_devices: a.conversation_devices(id).unwrap(),
        action: LifecycleAction::Transfer {
            target: target.device_id,
        },
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("a.sqlite");
    let mut a = ClientStore::create(&path, a, Zeroizing::new([1; 32])).unwrap();
    let input = a.queue_lifecycle(&context, &target).unwrap();
    drop(a);
    let mut a = ClientStore::open(&path, Zeroizing::new([1; 32])).unwrap();
    assert_eq!(a.pending_lifecycles().unwrap()[0].input, input);
    assert_eq!(a.queue_lifecycle(&context, &target).unwrap(), input);
    a.acknowledge_lifecycle(context.request_id).unwrap();
    assert_eq!(a.queue_lifecycle(&context, &target).unwrap(), input);
    let page = SyncPage {
        events: vec![StoredEvent {
            sequence: 1,
            sender: authority.device_id,
            event: input.commit,
            submitted_at: 1,
            expires_at: u64::MAX,
        }],
        next_cursor: 1,
        has_more: false,
    };
    let mut b = ClientStore::create(":memory:", b, Zeroizing::new([2; 32])).unwrap();
    assert!(
        b.apply_page_with_lifecycle_report(id, 0, &page, &[], |_| false, &[], &[])
            .is_err()
    );
    assert_eq!(b.cursor(id).unwrap(), 0);
    let approval = ApprovedLifecycle {
        context,
        authority,
        target,
    };
    b.apply_page_with_lifecycle_report(id, 0, &page, &[], |_| false, &[], &[approval])
        .unwrap();
    assert_eq!(b.cursor(id).unwrap(), 1);
}
