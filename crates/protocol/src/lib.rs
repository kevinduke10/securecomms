//! Versioned wire contracts. The relay never accepts plaintext chat content.
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const API_VERSION: u16 = 1;
pub const MAX_CIPHERTEXT_BYTES: usize = 64 * 1024;
pub const MAX_CONVERSATION_DEVICES: usize = 500;
// Keep worst-case JSON responses below Lambda's synchronous response limit.
pub const MAX_SYNC_PAGE: usize = 32;

macro_rules! id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}
id!(TenantId);
id!(DeviceId);
id!(ConversationId);
id!(MessageId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Member,
    Guest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub tenant_id: TenantId,
    pub device_id: DeviceId,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateConversation {
    pub id: ConversationId,
    /// Explicit device allowlist, including the creator. A guest cannot create groups.
    pub devices: Vec<DeviceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub tenant_id: TenantId,
    pub owner: DeviceId,
    pub devices: Vec<DeviceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Welcome,
    Commit,
    Application,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitEvent {
    pub protocol_version: u16,
    pub message_id: MessageId,
    pub kind: EventKind,
    /// Welcomes are addressed to the newly joining device only.
    pub recipient: Option<DeviceId>,
    pub ciphertext: String,
}

impl SubmitEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.protocol_version != API_VERSION {
            return Err("unsupported protocol version");
        }
        if (self.kind == EventKind::Welcome) != self.recipient.is_some() {
            return Err("only welcome events require a recipient");
        }
        if self.ciphertext.len() > MAX_CIPHERTEXT_BYTES.div_ceil(3) * 4 {
            return Err("ciphertext exceeds size limit");
        }
        let bytes = BASE64_STANDARD
            .decode(&self.ciphertext)
            .map_err(|_| "invalid ciphertext encoding")?;
        if bytes.is_empty() || bytes.len() > MAX_CIPHERTEXT_BYTES {
            return Err("invalid ciphertext size");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEvent {
    pub sequence: u64,
    pub sender: DeviceId,
    pub submitted_at: u64,
    pub expires_at: u64,
    #[serde(flatten)]
    pub event: SubmitEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedEvent {
    pub sequence: u64,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPage {
    pub events: Vec<StoredEvent>,
    /// Highest scanned sequence, including events not addressed to this device.
    pub next_cursor: u64,
    pub has_more: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

pub mod enrollment;
pub mod membership;
