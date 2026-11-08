//! Explicit, owner-authenticated group lifecycle transactions.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleAction {
    Transfer { target: DeviceId },
    Remove { target: DeviceId },
}
impl LifecycleAction {
    pub fn target(&self) -> DeviceId {
        match self {
            Self::Transfer { target } | Self::Remove { target } => *target,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleContext {
    pub conversation_id: ConversationId,
    pub request_id: MessageId,
    pub expected_owner: DeviceId,
    pub expected_devices: Vec<DeviceId>,
    pub action: LifecycleAction,
}
impl LifecycleContext {
    pub fn validate(&self) -> Result<(), &'static str> {
        let mut devices = self.expected_devices.clone();
        devices.sort();
        devices.dedup();
        if devices.len() != self.expected_devices.len()
            || devices.len() < 2
            || devices.len() > MAX_CONVERSATION_DEVICES
            || !devices.contains(&self.expected_owner)
            || !devices.contains(&self.action.target())
            || self.action.target() == self.expected_owner
        {
            return Err("invalid lifecycle transition");
        }
        Ok(())
    }
    pub fn aad(&self) -> Vec<u8> {
        [
            b"cirra-group-lifecycle-v1:".as_slice(),
            serde_json::to_vec(self)
                .expect("serializable context")
                .as_slice(),
        ]
        .concat()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupLifecycle {
    pub context: LifecycleContext,
    pub commit: SubmitEvent,
}
impl GroupLifecycle {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.context.validate()?;
        self.commit.validate()?;
        if self.commit.kind != EventKind::Commit
            || self.commit.recipient.is_some()
            || self.commit.message_id != self.context.request_id
        {
            return Err("invalid lifecycle commit");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedLifecycle {
    pub sequence: u64,
    pub duplicate: bool,
}
