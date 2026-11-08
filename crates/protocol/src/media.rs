//! Ciphertext object contracts. Only `ObjectUpload` may be sent to the relay.
use crate::{ConversationId, MessageId};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize};
pub const MAX_MEDIA_BYTES: usize = 25 * 1024 * 1024;
pub const MEDIA_HEADER: &[u8] = b"CIRRA-MEDIA-1\0";
pub const MEDIA_OVERHEAD: usize = MEDIA_HEADER.len() + 24 + 16;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectUpload {
    pub object_id: MessageId,
    pub ciphertext_size: u64,
    pub sha256: String,
}
impl ObjectUpload {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.ciphertext_size < MEDIA_OVERHEAD as u64
            || self.ciphertext_size > (MAX_MEDIA_BYTES + MEDIA_OVERHEAD) as u64
        {
            return Err("invalid ciphertext size");
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid ciphertext digest");
        }
        Ok(())
    }
}
/// Secret, authenticated application payload. Never log or expose to the relay/webview.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectAttachment {
    pub version: u16,
    pub conversation_id: ConversationId,
    pub object_id: MessageId,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub ciphertext_size: u64,
    pub key: String,
    pub sha256: String,
}
impl ObjectAttachment {
    pub fn upload(&self) -> ObjectUpload {
        ObjectUpload {
            object_id: self.object_id,
            ciphertext_size: self.ciphertext_size,
            sha256: self.sha256.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.upload().validate()?;
        if self.version != 1
            || self.size > MAX_MEDIA_BYTES as u64
            || self.ciphertext_size != self.size + MEDIA_OVERHEAD as u64
            || self.name.is_empty()
            || self.name.len() > 180
            || self
                .name
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\')
            || self.mime.len() > 100
            || self.mime.chars().any(char::is_control)
            || self.key.len() != 44
            || BASE64_STANDARD
                .decode(&self.key)
                .map_or(true, |v| v.len() != 32)
        {
            return Err("invalid object descriptor");
        }
        Ok(())
    }
    pub fn summary(&self) -> MediaSummary {
        MediaSummary {
            object_id: self.object_id,
            name: self.name.clone(),
            mime: self.mime.clone(),
            size: self.size,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaSummary {
    pub object_id: MessageId,
    pub name: String,
    pub mime: String,
    pub size: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transfer {
    pub url: String,
    pub headers: std::collections::BTreeMap<String, String>,
    pub expires_at: u64,
    pub requires_device_auth: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadGrant {
    pub transfer: Option<Transfer>,
    pub already_uploaded: bool,
}
