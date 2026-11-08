use super::*;
use croaker_protocol::lifecycle::*;
pub fn validate(
    actor: &Principal,
    group: &Conversation,
    id: ConversationId,
    input: &GroupLifecycle,
) -> Result<Conversation> {
    input.validate().map_err(StoreError::Invalid)?;
    if actor.device_id != group.owner
        || input.context.expected_owner != group.owner
        || input.context.conversation_id != id
    {
        return Err(StoreError::NotFound);
    }
    let mut expected = input.context.expected_devices.clone();
    expected.sort();
    let mut current = group.devices.clone();
    current.sort();
    if expected != current {
        return Err(StoreError::Conflict);
    }
    let mut next = group.clone();
    match input.context.action {
        LifecycleAction::Transfer { target } => next.owner = target,
        LifecycleAction::Remove { target } => next.devices.retain(|d| *d != target),
    };
    Ok(next)
}
pub fn fingerprint(input: &GroupLifecycle) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(input).expect("serializable lifecycle"),
    ))
}
