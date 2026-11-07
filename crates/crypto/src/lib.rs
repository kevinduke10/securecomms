//! Client-only MLS engine. Never link this crate into the relay.
//!
//! This foundation uses pinned, out-of-band device bindings. An authenticated
//! enrollment/key-directory workflow must supply those bindings in the product.
use std::collections::BTreeMap;

use ::tls_codec::{Deserialize as TlsDeserialize, Serialize as TlsSerialize};
use base64::{Engine, prelude::BASE64_STANDARD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use croaker_protocol::{ConversationId, DeviceId, MAX_CIPHERTEXT_BYTES, TenantId};
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::OpenMlsProvider;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const SUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519;
const VAULT_HEADER: &[u8] = b"CROAKER-DEVICE-1";
const MAX_VAULT: usize = 16 * 1024 * 1024;

/// Authenticate local history records with a caller-supplied row context. This
/// is separate from MLS transport encryption and from device-state snapshots.
pub fn seal_record(plaintext: &[u8], key: &[u8; 32], context: &[u8]) -> Result<Vec<u8>> {
    if plaintext.len() > MAX_VAULT {
        return Err(CryptoError::InvalidInput);
    }
    let mut nonce = [0; 24];
    OsRng.fill_bytes(&mut nonce);
    let aad = [b"croaker-local-record-v1".as_slice(), context].concat();
    let cipher = XChaCha20Poly1305::new(key.into());
    let encrypted = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::InvalidVault)?;
    Ok([nonce.as_slice(), &encrypted].concat())
}

pub fn open_record(bytes: &[u8], key: &[u8; 32], context: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if bytes.len() < 40 || bytes.len() > MAX_VAULT + 40 {
        return Err(CryptoError::InvalidVault);
    }
    let aad = [b"croaker-local-record-v1".as_slice(), context].concat();
    let cipher = XChaCha20Poly1305::new(key.into());
    Ok(Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&bytes[..24]),
                Payload {
                    msg: &bytes[24..],
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::InvalidVault)?,
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("MLS operation failed: {0}")]
    Mls(&'static str),
    #[error("device identity does not match the pinned binding")]
    IdentityMismatch,
    #[error("conversation not found")]
    UnknownConversation,
    #[error("invalid or oversized input")]
    InvalidInput,
    #[error("vault authentication failed or unsupported vault format")]
    InvalidVault,
}
type Result<T> = std::result::Result<T, CryptoError>;

fn checked<T, E>(result: std::result::Result<T, E>, operation: &'static str) -> Result<T> {
    result.map_err(|_| CryptoError::Mls(operation))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceBinding {
    pub tenant_id: TenantId,
    pub device_id: DeviceId,
    pub signature_key: Vec<u8>,
}

impl DeviceBinding {
    pub fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"croaker-device-binding-v1");
        hash.update(self.tenant_id.0.as_bytes());
        hash.update(self.device_id.0.as_bytes());
        hash.update(&self.signature_key);
        hex::encode(hash.finalize())
    }

    fn credential(&self) -> CredentialWithKey {
        CredentialWithKey {
            credential: BasicCredential::new(identity(self.tenant_id, self.device_id)).into(),
            signature_key: self.signature_key.clone().into(),
        }
    }
}

fn identity(tenant: TenantId, device: DeviceId) -> Vec<u8> {
    format!("croaker:v1:{tenant}:{device}").into_bytes()
}

fn group_id(tenant: TenantId, conversation: ConversationId) -> GroupId {
    GroupId::from_slice(format!("croaker:v1:{tenant}:{conversation}").as_bytes())
}

fn config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .ciphersuite(SUITE)
        .use_ratchet_tree_extension(true)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .build()
}

pub struct Device {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    binding: DeviceBinding,
    groups: BTreeMap<ConversationId, MlsGroup>,
}

pub struct MemberAdded {
    pub commit: Vec<u8>,
    pub welcome: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    Application {
        sender: DeviceId,
        plaintext: Vec<u8>,
    },
}

impl Device {
    pub fn new(tenant_id: TenantId, device_id: DeviceId) -> Result<Self> {
        let provider = OpenMlsRustCrypto::default();
        let signer = checked(
            SignatureKeyPair::new(SUITE.signature_algorithm()),
            "generate signer",
        )?;
        let binding = DeviceBinding {
            tenant_id,
            device_id,
            signature_key: signer.to_public_vec(),
        };
        Ok(Self {
            provider,
            signer,
            binding,
            groups: BTreeMap::new(),
        })
    }

    pub fn binding(&self) -> &DeviceBinding {
        &self.binding
    }

    /// Each generated key package is single-use and its private state stays on this device.
    pub fn key_package(&mut self) -> Result<Vec<u8>> {
        let bundle = checked(
            KeyPackage::builder().build(
                SUITE,
                &self.provider,
                &self.signer,
                self.binding.credential(),
            ),
            "generate key package",
        )?;
        checked(
            bundle.key_package().tls_serialize_detached(),
            "serialize key package",
        )
    }

    pub fn create_conversation(&mut self, conversation: ConversationId) -> Result<()> {
        if self.groups.contains_key(&conversation) {
            return Err(CryptoError::InvalidInput);
        }
        let group = checked(
            MlsGroup::new_with_group_id(
                &self.provider,
                &self.signer,
                &config(),
                group_id(self.binding.tenant_id, conversation),
                self.binding.credential(),
            ),
            "create group",
        )?;
        self.groups.insert(conversation, group);
        Ok(())
    }

    /// `peer` must come from a verified device enrollment or an out-of-band comparison.
    pub fn add_member(
        &mut self,
        conversation: ConversationId,
        package: &[u8],
        peer: &DeviceBinding,
    ) -> Result<MemberAdded> {
        self.add_members(conversation, &[(package, peer)])
    }

    pub fn add_members(
        &mut self,
        conversation: ConversationId,
        members: &[(&[u8], &DeviceBinding)],
    ) -> Result<MemberAdded> {
        if members.is_empty() {
            return Err(CryptoError::InvalidInput);
        }
        let mut packages = Vec::new();
        for (bytes, peer) in members {
            if peer.tenant_id != self.binding.tenant_id || bytes.len() > MAX_CIPHERTEXT_BYTES {
                return Err(CryptoError::IdentityMismatch);
            }
            let package = checked(
                KeyPackageIn::tls_deserialize_exact(bytes),
                "parse key package",
            )?;
            let package = checked(
                package.validate(self.provider.crypto(), ProtocolVersion::Mls10),
                "validate key package",
            )?;
            let leaf = package.leaf_node();
            if leaf.credential() != &peer.credential().credential
                || leaf.signature_key().as_slice() != peer.signature_key
            {
                return Err(CryptoError::IdentityMismatch);
            }
            packages.push(package);
        }
        let group = self
            .groups
            .get_mut(&conversation)
            .ok_or(CryptoError::UnknownConversation)?;
        let (commit, welcome, _) = checked(
            group.add_members(&self.provider, &self.signer, &packages),
            "add member",
        )?;
        let result = MemberAdded {
            commit: checked(commit.tls_serialize_detached(), "serialize commit")?,
            welcome: checked(welcome.tls_serialize_detached(), "serialize welcome")?,
        };
        checked(group.merge_pending_commit(&self.provider), "merge commit")?;
        Ok(result)
    }

    /// A welcome is accepted only when every member's credential and signing key
    /// matches a pinned binding. A relay-supplied display name is never identity proof.
    pub fn join(
        &mut self,
        conversation: ConversationId,
        welcome: &[u8],
        peers: &[DeviceBinding],
    ) -> Result<()> {
        if self.groups.contains_key(&conversation) || welcome.len() > MAX_CIPHERTEXT_BYTES {
            return Err(CryptoError::InvalidInput);
        }
        let message = checked(
            MlsMessageIn::tls_deserialize_exact(welcome),
            "parse welcome",
        )?;
        let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
            return Err(CryptoError::InvalidInput);
        };
        let staged = checked(
            StagedWelcome::new_from_welcome(&self.provider, config().join_config(), welcome, None),
            "stage welcome",
        )?;
        let matches = staged.group_context().group_id()
            == &group_id(self.binding.tenant_id, conversation)
            && staged.members().all(|member| {
                peers
                    .iter()
                    .chain(std::iter::once(&self.binding))
                    .any(|peer| {
                        peer.tenant_id == self.binding.tenant_id
                            && member.credential == peer.credential().credential
                            && member.signature_key == peer.signature_key
                    })
            });
        if !matches {
            return Err(CryptoError::IdentityMismatch);
        }
        let group = checked(staged.into_group(&self.provider), "join group")?;
        self.groups.insert(conversation, group);
        Ok(())
    }

    pub fn encrypt(&mut self, conversation: ConversationId, plaintext: &[u8]) -> Result<Vec<u8>> {
        // Leave space for authenticated MLS framing inside the transport limit.
        if plaintext.len() > MAX_CIPHERTEXT_BYTES - 4096 {
            return Err(CryptoError::InvalidInput);
        }
        let group = self
            .groups
            .get_mut(&conversation)
            .ok_or(CryptoError::UnknownConversation)?;
        let message = checked(
            group.create_message(&self.provider, &self.signer, plaintext),
            "encrypt message",
        )?;
        checked(message.tls_serialize_detached(), "serialize message")
    }

    /// Rotate group keys after an explicitly authorized removal. The caller must
    /// persist the resulting device state and outgoing commit atomically.
    pub fn remove_member(
        &mut self,
        conversation: ConversationId,
        device: DeviceId,
    ) -> Result<Vec<u8>> {
        if device == self.binding.device_id {
            return Err(CryptoError::InvalidInput);
        }
        let group = self
            .groups
            .get_mut(&conversation)
            .ok_or(CryptoError::UnknownConversation)?;
        let credential: Credential =
            BasicCredential::new(identity(self.binding.tenant_id, device)).into();
        let index = group
            .members()
            .find(|member| member.credential == credential)
            .map(|m| m.index)
            .ok_or(CryptoError::IdentityMismatch)?;
        let (commit, _, _) = checked(
            group.remove_members(&self.provider, &self.signer, &[index]),
            "remove member",
        )?;
        let bytes = checked(commit.tls_serialize_detached(), "serialize removal")?;
        checked(group.merge_pending_commit(&self.provider), "merge removal")?;
        Ok(bytes)
    }

    /// Accept only a removal authored by the pinned conversation authority and
    /// matching the explicitly approved device. Do not infer approval from
    /// unauthenticated relay metadata. Other membership changes remain rejected.
    pub fn apply_removal(
        &mut self,
        conversation: ConversationId,
        ciphertext: &[u8],
        authority: &DeviceBinding,
        removed: DeviceId,
    ) -> Result<()> {
        if authority.tenant_id != self.binding.tenant_id || ciphertext.len() > MAX_CIPHERTEXT_BYTES
        {
            return Err(CryptoError::IdentityMismatch);
        }
        let message = checked(
            MlsMessageIn::tls_deserialize_exact(ciphertext),
            "parse removal",
        )?;
        let message = checked(message.try_into_protocol_message(), "validate removal type")?;
        let group = self
            .groups
            .get_mut(&conversation)
            .ok_or(CryptoError::UnknownConversation)?;
        let members: Vec<_> = group.members().collect();
        let authority_index = members
            .iter()
            .find(|member| {
                member.credential == authority.credential().credential
                    && member.signature_key == authority.signature_key
            })
            .map(|m| m.index)
            .ok_or(CryptoError::IdentityMismatch)?;
        let removed_credential: Credential =
            BasicCredential::new(identity(self.binding.tenant_id, removed)).into();
        let removed_index = members
            .iter()
            .find(|member| member.credential == removed_credential)
            .map(|m| m.index)
            .ok_or(CryptoError::IdentityMismatch)?;
        let processed = checked(
            group.process_message(&self.provider, message),
            "authenticate removal",
        )?;
        if processed.sender() != &Sender::Member(authority_index) {
            return Err(CryptoError::IdentityMismatch);
        }
        let ProcessedMessageContent::StagedCommitMessage(commit) = processed.into_content() else {
            return Err(CryptoError::InvalidInput);
        };
        let mut proposals = commit.queued_proposals();
        if !matches!(proposals.next().map(|p| p.proposal()), Some(Proposal::Remove(p)) if p.removed() == removed_index)
            || proposals.next().is_some()
            || commit.update_path_leaf_node().is_some_and(|leaf| {
                leaf.credential() != &authority.credential().credential
                    || leaf.signature_key().as_slice() != authority.signature_key
            })
        {
            return Err(CryptoError::IdentityMismatch);
        }
        drop(proposals);
        checked(
            group.merge_staged_commit(&self.provider, *commit),
            "merge authorized removal",
        )?;
        Ok(())
    }

    pub fn receive(&mut self, conversation: ConversationId, ciphertext: &[u8]) -> Result<Received> {
        if ciphertext.len() > MAX_CIPHERTEXT_BYTES {
            return Err(CryptoError::InvalidInput);
        }
        let message = checked(
            MlsMessageIn::tls_deserialize_exact(ciphertext),
            "parse message",
        )?;
        let message = checked(message.try_into_protocol_message(), "validate message type")?;
        let group = self
            .groups
            .get_mut(&conversation)
            .ok_or(CryptoError::UnknownConversation)?;
        let processed = checked(
            group.process_message(&self.provider, message),
            "authenticate message",
        )?;
        let credential = BasicCredential::try_from(processed.credential().clone())
            .map_err(|_| CryptoError::IdentityMismatch)?;
        let prefix = format!("croaker:v1:{}:", self.binding.tenant_id);
        let identity = std::str::from_utf8(credential.identity())
            .map_err(|_| CryptoError::IdentityMismatch)?;
        let sender = identity
            .strip_prefix(&prefix)
            .and_then(|s| s.parse().ok())
            .ok_or(CryptoError::IdentityMismatch)?;
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(message) => Ok(Received::Application {
                sender,
                plaintext: message.into_bytes(),
            }),
            // Business membership authorization is a separate layer. Do not
            // automatically merge arbitrary member-supplied changes here.
            _ => Err(CryptoError::Mls(
                "membership changes require explicit approval",
            )),
        }
    }

    /// Export an authenticated encrypted snapshot, including current MLS state.
    /// This is a DEVICE VAULT, not a history backup: never restore it on a second
    /// active device or roll it back after sending. The client transaction layer
    /// must commit this snapshot and its outbox atomically before network I/O.
    pub fn seal(&self, key: &[u8; 32]) -> Result<Vec<u8>> {
        let values = self
            .provider
            .storage()
            .values
            .read()
            .map_err(|_| CryptoError::InvalidVault)?;
        let snapshot = Snapshot {
            binding: self.binding.clone(),
            signer: &self.signer,
            conversations: self.groups.keys().copied().collect(),
            storage: values
                .iter()
                .map(|(k, v)| (BASE64_STANDARD.encode(k), BASE64_STANDARD.encode(v)))
                .collect(),
        };
        let plaintext =
            Zeroizing::new(serde_json::to_vec(&snapshot).map_err(|_| CryptoError::InvalidVault)?);
        if plaintext.len() > MAX_VAULT {
            return Err(CryptoError::InvalidVault);
        }
        let mut nonce = [0; 24];
        OsRng.fill_bytes(&mut nonce);
        let cipher = XChaCha20Poly1305::new(key.into());
        let encrypted = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: VAULT_HEADER,
                },
            )
            .map_err(|_| CryptoError::InvalidVault)?;
        Ok([VAULT_HEADER, &nonce, &encrypted].concat())
    }

    pub fn open(bytes: &[u8], key: &[u8; 32]) -> Result<Self> {
        let offset = VAULT_HEADER.len();
        if bytes.len() < offset + 24 + 16
            || bytes.len() > MAX_VAULT + offset + 40
            || !bytes.starts_with(VAULT_HEADER)
        {
            return Err(CryptoError::InvalidVault);
        }
        let cipher = XChaCha20Poly1305::new(key.into());
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    XNonce::from_slice(&bytes[offset..offset + 24]),
                    Payload {
                        msg: &bytes[offset + 24..],
                        aad: VAULT_HEADER,
                    },
                )
                .map_err(|_| CryptoError::InvalidVault)?,
        );
        let snapshot: OwnedSnapshot =
            serde_json::from_slice(&plaintext).map_err(|_| CryptoError::InvalidVault)?;
        if snapshot.signer.to_public_vec() != snapshot.binding.signature_key {
            return Err(CryptoError::InvalidVault);
        }
        let provider = OpenMlsRustCrypto::default();
        {
            let mut values = provider
                .storage()
                .values
                .write()
                .map_err(|_| CryptoError::InvalidVault)?;
            for (k, v) in snapshot.storage {
                values.insert(
                    BASE64_STANDARD
                        .decode(k)
                        .map_err(|_| CryptoError::InvalidVault)?,
                    BASE64_STANDARD
                        .decode(v)
                        .map_err(|_| CryptoError::InvalidVault)?,
                );
            }
        }
        let mut groups = BTreeMap::new();
        for id in snapshot.conversations {
            let group = checked(
                MlsGroup::load(
                    provider.storage(),
                    &group_id(snapshot.binding.tenant_id, id),
                ),
                "load group",
            )?
            .ok_or(CryptoError::InvalidVault)?;
            groups.insert(id, group);
        }
        Ok(Self {
            provider,
            signer: snapshot.signer,
            binding: snapshot.binding,
            groups,
        })
    }
}

#[derive(Serialize)]
struct Snapshot<'a> {
    binding: DeviceBinding,
    signer: &'a SignatureKeyPair,
    conversations: Vec<ConversationId>,
    storage: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct OwnedSnapshot {
    binding: DeviceBinding,
    signer: SignatureKeyPair,
    conversations: Vec<ConversationId>,
    storage: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Device, Device, ConversationId) {
        let tenant = TenantId::new();
        let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
        let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
        let id = ConversationId::new();
        alice.create_conversation(id).unwrap();
        let package = bob.key_package().unwrap();
        let added = alice.add_member(id, &package, bob.binding()).unwrap();
        bob.join(id, &added.welcome, &[alice.binding().clone()])
            .unwrap();
        (alice, bob, id)
    }

    #[test]
    fn encrypted_exchange_replay_tampering_and_vault_restart() {
        let (mut alice, mut bob, id) = pair();
        let secret = b"confidential business message";
        let ciphertext = alice.encrypt(id, secret).unwrap();
        assert!(!ciphertext.windows(secret.len()).any(|w| w == secret));
        let mut tampered = ciphertext.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(bob.receive(id, &tampered).is_err());
        assert_eq!(
            bob.receive(id, &ciphertext).unwrap(),
            Received::Application {
                sender: alice.binding.device_id,
                plaintext: secret.to_vec()
            }
        );
        assert!(bob.receive(id, &ciphertext).is_err());
        let key = [42; 32];
        let vault = bob.seal(&key).unwrap();
        assert!(Device::open(&vault, &[43; 32]).is_err());
        let mut bob = Device::open(&vault, &key).unwrap();
        let reply = bob.encrypt(id, b"received").unwrap();
        assert!(
            matches!(alice.receive(id, &reply).unwrap(), Received::Application { plaintext, .. } if plaintext == b"received")
        );
        let mut corrupted = vault;
        *corrupted.last_mut().unwrap() ^= 1;
        assert!(Device::open(&corrupted, &key).is_err());
    }

    #[test]
    fn rejects_unpinned_identity_wrong_tenant_and_wrong_conversation() {
        let (mut alice, mut bob, id) = pair();
        let mut impostor = Device::new(alice.binding.tenant_id, bob.binding.device_id).unwrap();
        let package = impostor.key_package().unwrap();
        assert!(matches!(
            alice.add_member(id, &package, bob.binding()),
            Err(CryptoError::IdentityMismatch)
        ));
        let mut outsider = Device::new(TenantId::new(), DeviceId::new()).unwrap();
        let package = outsider.key_package().unwrap();
        assert!(alice.add_member(id, &package, outsider.binding()).is_err());
        let other = ConversationId::new();
        alice.create_conversation(other).unwrap();
        let ciphertext = alice.encrypt(id, b"bound to group").unwrap();
        assert!(bob.receive(other, &ciphertext).is_err());
    }

    #[test]
    fn three_member_group_uses_pinned_roster() {
        let tenant = TenantId::new();
        let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
        let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
        let mut charlie = Device::new(tenant, DeviceId::new()).unwrap();
        let id = ConversationId::new();
        alice.create_conversation(id).unwrap();
        let bob_package = bob.key_package().unwrap();
        let charlie_package = charlie.key_package().unwrap();
        let added = alice
            .add_members(
                id,
                &[
                    (&bob_package, bob.binding()),
                    (&charlie_package, charlie.binding()),
                ],
            )
            .unwrap();
        bob.join(
            id,
            &added.welcome,
            &[alice.binding().clone(), charlie.binding().clone()],
        )
        .unwrap();
        charlie
            .join(
                id,
                &added.welcome,
                &[alice.binding().clone(), bob.binding().clone()],
            )
            .unwrap();
        let message = alice.encrypt(id, b"hello team").unwrap();
        for device in [&mut bob, &mut charlie] {
            assert!(
                matches!(device.receive(id, &message).unwrap(), Received::Application { plaintext, .. } if plaintext == b"hello team")
            );
        }
        let before_removal = charlie.seal(&[91; 32]).unwrap();
        let removal = alice.remove_member(id, bob.binding.device_id).unwrap();
        assert!(
            charlie
                .apply_removal(id, &removal, bob.binding(), bob.binding.device_id)
                .is_err()
        );
        // The application transaction layer rolls back failed operations.
        let mut charlie = Device::open(&before_removal, &[91; 32]).unwrap();
        charlie
            .apply_removal(id, &removal, alice.binding(), bob.binding.device_id)
            .unwrap();
        let new_message = alice.encrypt(id, b"only current members").unwrap();
        assert!(bob.receive(id, &new_message).is_err());
        assert!(
            matches!(charlie.receive(id, &new_message).unwrap(), Received::Application { plaintext, .. } if plaintext == b"only current members")
        );
    }
}
