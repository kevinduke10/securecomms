//! Owner-authorized, atomic admission of one already enrolled device.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupAdmission {
    pub request_id: MessageId,
    pub expected_devices: Vec<DeviceId>,
    pub added_device: DeviceId,
    pub commit: SubmitEvent,
    pub welcome: SubmitEvent,
}
impl GroupAdmission {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.commit.validate()?;
        self.welcome.validate()?;
        let mut expected = self.expected_devices.clone();
        expected.sort();
        expected.dedup();
        if expected.len() != self.expected_devices.len()
            || expected.is_empty()
            || expected.len() >= MAX_CONVERSATION_DEVICES
            || expected.contains(&self.added_device)
            || self.commit.kind != EventKind::Commit
            || self.commit.recipient.is_some()
            || self.welcome.kind != EventKind::Welcome
            || self.welcome.recipient != Some(self.added_device)
            || self.commit.message_id == self.welcome.message_id
        {
            return Err("invalid group admission");
        }
        let commit = BASE64_STANDARD
            .decode(&self.commit.ciphertext)
            .map_err(|_| "invalid commit encoding")?;
        let welcome = BASE64_STANDARD
            .decode(&self.welcome.ciphertext)
            .map_err(|_| "invalid welcome encoding")?;
        if commit.len() + welcome.len() > MAX_CIPHERTEXT_BYTES {
            return Err("admission ciphertext exceeds size limit");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedAdmission {
    pub commit_sequence: u64,
    pub welcome_sequence: u64,
    pub duplicate: bool,
}
