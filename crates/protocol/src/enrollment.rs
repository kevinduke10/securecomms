//! Public enrollment proof format. No private MLS material belongs here.
use super::*;
use sha2::{Digest, Sha256};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityBinding {
    pub tenant_id: TenantId,
    pub device_id: DeviceId,
    pub signature_key: Vec<u8>,
}
impl IdentityBinding {
    pub fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"croaker-device-binding-v1");
        hash.update(self.tenant_id.0.as_bytes());
        hash.update(self.device_id.0.as_bytes());
        hash.update(&self.signature_key);
        hex::encode(hash.finalize())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentInput {
    pub id: MessageId,
    pub binding: IdentityBinding,
    pub label: String,
    /// Digest of the random device credential retained by the candidate client.
    pub token_hash: String,
    pub proof: Vec<u8>,
}
impl EnrollmentInput {
    pub fn signing_bytes(&self, account_id: &str) -> Vec<u8> {
        // Length-prefixed account ID; remaining fields have fixed encodings or
        // are hashed, avoiding ambiguous concatenation and JSON canonicalization.
        let mut bytes = b"croaker-device-enrollment-v1\0".to_vec();
        bytes.extend_from_slice(&(account_id.len() as u32).to_be_bytes());
        bytes.extend_from_slice(account_id.as_bytes());
        bytes.extend_from_slice(self.id.0.as_bytes());
        bytes.extend_from_slice(self.binding.tenant_id.0.as_bytes());
        bytes.extend_from_slice(self.binding.device_id.0.as_bytes());
        bytes.extend_from_slice(&self.binding.signature_key);
        bytes.extend_from_slice(Sha256::digest(self.label.as_bytes()).as_slice());
        bytes.extend_from_slice(self.token_hash.as_bytes());
        bytes
    }
}
