use croaker_crypto::{Device, Received};
use croaker_protocol::{ConversationId, DeviceId, MessageId, TenantId, lifecycle::*};
#[test]
fn transfer_and_removal_are_authenticated_and_rotate_keys() {
    let tenant = TenantId::new();
    let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
    let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
    let mut carol = Device::new(tenant, DeviceId::new()).unwrap();
    let id = ConversationId::new();
    alice.create_conversation(id).unwrap();
    let bp = bob.key_package().unwrap();
    let cp = carol.key_package().unwrap();
    let a = alice
        .add_members(id, &[(&bp, bob.binding()), (&cp, carol.binding())])
        .unwrap();
    bob.join(
        id,
        &a.welcome,
        &[alice.binding().clone(), carol.binding().clone()],
    )
    .unwrap();
    carol
        .join(
            id,
            &a.welcome,
            &[alice.binding().clone(), bob.binding().clone()],
        )
        .unwrap();
    let context = LifecycleContext {
        conversation_id: id,
        request_id: MessageId::new(),
        expected_owner: alice.binding().device_id,
        expected_devices: alice.conversation_devices(id).unwrap(),
        action: LifecycleAction::Transfer {
            target: bob.binding().device_id,
        },
    };
    let before = carol.seal(&[7; 32]).unwrap();
    let commit = alice.commit_lifecycle(&context, bob.binding()).unwrap();
    let mut altered = context.clone();
    altered.action = LifecycleAction::Transfer {
        target: carol.binding().device_id,
    };
    assert!(
        carol
            .apply_lifecycle(&altered, &commit, alice.binding(), &carol.binding().clone())
            .is_err()
    );
    carol = Device::open(&before, &[7; 32]).unwrap();
    carol
        .apply_lifecycle(&context, &commit, alice.binding(), bob.binding())
        .unwrap();
    bob.apply_lifecycle(&context, &commit, alice.binding(), &bob.binding().clone())
        .unwrap();
    let remove = LifecycleContext {
        conversation_id: id,
        request_id: MessageId::new(),
        expected_owner: bob.binding().device_id,
        expected_devices: bob.conversation_devices(id).unwrap(),
        action: LifecycleAction::Remove {
            target: alice.binding().device_id,
        },
    };
    let commit = bob.commit_lifecycle(&remove, alice.binding()).unwrap();
    carol
        .apply_lifecycle(&remove, &commit, bob.binding(), alice.binding())
        .unwrap();
    let ciphertext = bob.encrypt(id, b"after removal").unwrap();
    assert!(alice.receive(id, &ciphertext).is_err());
    assert!(
        matches!(carol.receive(id,&ciphertext).unwrap(),Received::Application{plaintext,..} if plaintext==b"after removal")
    );
}
