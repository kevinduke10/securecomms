//! Account grants and pending public identities. Independent from MLS secrets.
use crate::*;
use croaker_protocol::enrollment::*;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const ENROLLMENT_TTL: u64 = 600;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccountGrant {
    pub issuer: String,
    pub subject: String,
    pub tenant_id: TenantId,
    pub role: Role,
    pub name: String,
    #[serde(default)]
    pub bootstrap_device: Option<IdentityBinding>,
}
impl AccountGrant {
    pub fn id(&self) -> String {
        identity_id(&self.issuer, &self.subject)
    }
    pub fn validate(&self) -> Result<()> {
        if self
            .bootstrap_device
            .as_ref()
            .is_some_and(|b| b.tenant_id != self.tenant_id || b.signature_key.len() != 32)
        {
            return Err(StoreError::Invalid("invalid initial device"));
        }
        if self.issuer.len() > 300
            || !self.issuer.starts_with("https://")
            || self.subject.is_empty()
            || self.subject.len() > 128
            || self.name.is_empty()
            || self.name.len() > 100
        {
            return Err(StoreError::Invalid("invalid account grant"));
        }
        Ok(())
    }
}
pub fn identity_id(issuer: &str, subject: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"croaker-account-v1\0");
    digest.update((issuer.len() as u32).to_be_bytes());
    digest.update(issuer.as_bytes());
    digest.update(subject.as_bytes());
    hex::encode(digest.finalize())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentStatus {
    Pending,
    Approved,
    Rejected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Enrollment {
    pub input: EnrollmentInput,
    pub created_at: u64,
    pub expires_at: u64,
    pub status: EnrollmentStatus,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovedDevice {
    pub binding: IdentityBinding,
    pub label: String,
    pub approved_at: u64,
    pub approved_by: DeviceId,
    #[serde(default)]
    pub revoked: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountRecord {
    pub grant: AccountGrant,
    pub disabled: bool,
    pub devices: Vec<ApprovedDevice>,
    pub requests: Vec<Enrollment>,
}
impl AccountRecord {
    pub fn new(grant: AccountGrant) -> Result<Self> {
        grant.validate()?;
        let devices = grant
            .bootstrap_device
            .as_ref()
            .map(|binding| ApprovedDevice {
                binding: binding.clone(),
                label: "Initial device (operator verified)".into(),
                approved_at: now(),
                approved_by: binding.device_id,
                revoked: false,
            })
            .into_iter()
            .collect();
        Ok(Self {
            grant,
            disabled: false,
            devices,
            requests: vec![],
        })
    }
    pub fn active(&self) -> Result<()> {
        if self.disabled {
            Err(StoreError::Unauthorized)
        } else {
            Ok(())
        }
    }
    pub fn request(&mut self, input: EnrollmentInput) -> Result<()> {
        self.active()?;
        if input.binding.tenant_id != self.grant.tenant_id
            || input.binding.signature_key.len() != 32
            || input.proof.len() != 64
            || input.label.is_empty()
            || input.label.len() > 80
            || input.label.chars().any(char::is_control)
            || input.token_hash.len() != 64
            || !input.token_hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(StoreError::Invalid("invalid enrollment"));
        }
        let key = VerifyingKey::from_bytes(
            input
                .binding
                .signature_key
                .as_slice()
                .try_into()
                .map_err(|_| StoreError::Invalid("invalid identity key"))?,
        )
        .map_err(|_| StoreError::Invalid("invalid identity key"))?;
        let signature =
            Signature::from_slice(&input.proof).map_err(|_| StoreError::Unauthorized)?;
        key.verify_strict(&input.signing_bytes(&self.grant.id()), &signature)
            .map_err(|_| StoreError::Unauthorized)?;
        if let Some(previous) = self.requests.iter().find(|r| r.input.id == input.id) {
            if serde_json::to_vec(&previous.input).ok() != serde_json::to_vec(&input).ok() {
                return Err(StoreError::Conflict);
            }
            return Ok(());
        }
        if self
            .devices
            .iter()
            .any(|d| d.binding.device_id == input.binding.device_id)
            || self
                .requests
                .iter()
                .any(|r| r.input.binding.device_id == input.binding.device_id)
        {
            return Err(StoreError::Conflict);
        }
        let timestamp = now();
        self.requests
            .retain(|r| r.expires_at > timestamp && r.status == EnrollmentStatus::Pending);
        if self.requests.len() >= 5 || self.devices.len() >= 10 {
            return Err(StoreError::Invalid(
                "device or pending request limit reached",
            ));
        }
        self.requests.push(Enrollment {
            input,
            created_at: timestamp,
            expires_at: timestamp + ENROLLMENT_TTL,
            status: EnrollmentStatus::Pending,
        });
        Ok(())
    }
    pub fn can_approve(&self, actor: &Principal) -> Result<()> {
        self.active()?;
        if actor.tenant_id != self.grant.tenant_id
            || (actor.role != Role::Admin
                && !self
                    .devices
                    .iter()
                    .any(|d| d.binding.device_id == actor.device_id))
        {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
    pub fn decide(
        &mut self,
        actor: &Principal,
        id: MessageId,
        fingerprint: &str,
        approve: bool,
    ) -> Result<Option<(Principal, String)>> {
        self.can_approve(actor)?;
        let request = self
            .requests
            .iter_mut()
            .find(|r| r.input.id == id)
            .ok_or(StoreError::NotFound)?;
        if request.input.binding.fingerprint() != fingerprint {
            return Err(StoreError::Conflict);
        }
        if request.status == EnrollmentStatus::Approved
            && approve
            && self
                .devices
                .iter()
                .any(|d| d.binding == request.input.binding && d.approved_by == actor.device_id)
        {
            return Ok(None);
        }
        if request.status == EnrollmentStatus::Rejected && !approve {
            return Ok(None);
        }
        if request.status != EnrollmentStatus::Pending {
            return Err(StoreError::Conflict);
        }
        if request.expires_at <= now() {
            return Err(StoreError::Invalid("enrollment expired; start again"));
        }
        if !approve {
            request.status = EnrollmentStatus::Rejected;
            return Ok(None);
        }
        if actor.device_id == request.input.binding.device_id || self.devices.len() >= 10 {
            return Err(StoreError::Conflict);
        }
        request.status = EnrollmentStatus::Approved;
        self.devices.push(ApprovedDevice {
            binding: request.input.binding.clone(),
            label: request.input.label.clone(),
            approved_at: now(),
            approved_by: actor.device_id,
            revoked: false,
        });
        Ok(Some((
            Principal {
                tenant_id: self.grant.tenant_id,
                device_id: request.input.binding.device_id,
                role: self.grant.role,
            },
            request.input.token_hash.clone(),
        )))
    }
    /// Never return token digests or proofs in the account UI response.
    pub fn public(&self) -> serde_json::Value {
        serde_json::json!({"id":self.grant.id(),"tenant_id":self.grant.tenant_id,"name":self.grant.name,"role":self.grant.role,"devices":self.devices,"requests":self.requests.iter().map(|r|serde_json::json!({"id":r.input.id,"binding":r.input.binding,"fingerprint":r.input.binding.fingerprint(),"label":r.input.label,"status":r.status,"created_at":r.created_at,"expires_at":r.expires_at})).collect::<Vec<_>>()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn expired_and_rejected_requests_never_become_approved() {
        let tenant = TenantId::new();
        let actor = Principal {
            tenant_id: tenant,
            device_id: DeviceId::new(),
            role: Role::Admin,
        };
        let mut account = AccountRecord::new(AccountGrant {
            issuer: "https://fixture.example".into(),
            subject: "fixture".into(),
            tenant_id: tenant,
            role: Role::Guest,
            name: "Guest".into(),
            bootstrap_device: None,
        })
        .unwrap();
        let key = SigningKey::from_bytes(&[4; 32]);
        let mut input = EnrollmentInput {
            id: MessageId::new(),
            binding: IdentityBinding {
                tenant_id: tenant,
                device_id: DeviceId::new(),
                signature_key: key.verifying_key().to_bytes().to_vec(),
            },
            label: "Fixture".into(),
            token_hash: "a".repeat(64),
            proof: vec![],
        };
        input.proof = key
            .sign(&input.signing_bytes(&account.grant.id()))
            .to_bytes()
            .to_vec();
        account.request(input.clone()).unwrap();
        account.requests[0].expires_at = now() - 1;
        assert!(
            account
                .decide(&actor, input.id, &input.binding.fingerprint(), true)
                .is_err()
        );
        assert!(account.devices.is_empty());
        account.requests[0].expires_at = now() + 600;
        account
            .decide(&actor, input.id, &input.binding.fingerprint(), false)
            .unwrap();
        assert!(
            account
                .decide(&actor, input.id, &input.binding.fingerprint(), true)
                .is_err()
        );
        assert!(account.devices.is_empty());
    }
}
