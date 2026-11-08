//! Portable HISTORY ONLY recovery. Never serialize Device, MLS state, credentials,
//! cursors, or an outbox here. An archive cannot authorize a new live device.
use crate::ClientStore;
use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_crypto::{open_record, seal_record};
use croaker_protocol::{ConversationId, DeviceId, MessageId, TenantId};
use hkdf::Hkdf;
use rand::{RngCore, rngs::OsRng};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use zeroize::Zeroizing;

const MAGIC: &[u8] = b"CROAKER-HISTORY-1\0";
const WRAPPED_LEN: usize = 24 + 32 + 16;
const PREFIX: usize = MAGIC.len() + 32;
const HEADER: usize = PREFIX + WRAPPED_LEN;
pub const MAX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BACKUP_BYTES: usize = MAX_ARCHIVE_BYTES + HEADER + 40;

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("Invalid recovery code. Check every group and the checksum.")]
    Code,
    #[error("Backup could not be opened. Check the recovery code and file; nothing was imported.")]
    Invalid,
    #[error("History backups must be 16 MiB or smaller.")]
    Size,
    #[error("The recovered archive could not be saved.")]
    Storage,
}
type Result<T> = std::result::Result<T, BackupError>;

// Deliberately no Debug/Serialize: secrets cannot accidentally enter logs or manifests.
pub struct RecoveryCode(Zeroizing<[u8; 32]>);
impl Default for RecoveryCode {
    fn default() -> Self {
        Self::generate()
    }
}
impl RecoveryCode {
    pub fn generate() -> Self {
        let mut key = Zeroizing::new([0; 32]);
        OsRng.fill_bytes(key.as_mut());
        Self(key)
    }
    pub fn display(&self) -> Zeroizing<String> {
        let encoded = Zeroizing::new(hex::encode(*self.0));
        let checksum = hex::encode(&Sha256::digest(self.0.as_slice())[..4]);
        Zeroizing::new(format!(
            "CRK1-{}-{checksum}",
            encoded
                .as_bytes()
                .chunks(8)
                .map(|part| std::str::from_utf8(part).unwrap())
                .collect::<Vec<_>>()
                .join("-")
        ))
    }
    pub fn parse(value: &str) -> Result<Self> {
        if value.len() > 128 {
            return Err(BackupError::Code);
        }
        let normalized = Zeroizing::new(value.trim().to_ascii_lowercase());
        let parts: Vec<_> = normalized.split('-').collect();
        if parts.len() != 10 || parts[0] != "crk1" || parts[1..].iter().any(|p| p.len() != 8) {
            return Err(BackupError::Code);
        }
        let encoded = Zeroizing::new(parts[1..9].concat());
        let mut key = Zeroizing::new([0; 32]);
        hex::decode_to_slice(encoded.as_bytes(), key.as_mut()).map_err(|_| BackupError::Code)?;
        if hex::encode(&Sha256::digest(key.as_slice())[..4]) != parts[9] {
            return Err(BackupError::Code);
        }
        Ok(Self(key))
    }
    fn wrapping_key(&self, salt: &[u8]) -> Zeroizing<[u8; 32]> {
        let mut key = Zeroizing::new([0; 32]);
        Hkdf::<Sha256>::new(Some(salt), self.0.as_slice())
            .expand(b"croaker-history-backup-wrapping-v1", key.as_mut())
            .expect("valid HKDF size");
        key
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Person {
    pub device_id: DeviceId,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data: String,
    pub size: usize,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reaction {
    pub sender: DeviceId,
    pub emoji: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageRef {
    pub message_id: MessageId,
    pub sender: DeviceId,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub id: MessageId,
    pub sender: DeviceId,
    pub sent_at: u64,
    pub text: String,
    pub attachment: Option<Attachment>,
    pub reactions: Vec<Reaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageRef>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub id: ConversationId,
    pub title: String,
    pub kind: String,
    pub members: Vec<DeviceId>,
    pub messages: Vec<Message>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    pub version: u16,
    pub id: MessageId,
    pub created_at: u64,
    pub tenant_id: TenantId,
    pub source_device: DeviceId,
    pub people: Vec<Person>,
    pub conversations: Vec<Conversation>,
}
impl Archive {
    pub fn validate(&self) -> Result<()> {
        let invalid = || BackupError::Invalid;
        if self.version != 1
            || self.created_at > 253_402_300_799
            || self.people.len() > 4096
            || self.conversations.len() > 1000
        {
            return Err(invalid());
        }
        let mut people = HashSet::new();
        for person in &self.people {
            if person.name.is_empty() || person.name.len() > 200 || !people.insert(person.device_id)
            {
                return Err(invalid());
            }
        }
        if !people.contains(&self.source_device) {
            return Err(invalid());
        }
        let mut conversations = HashSet::new();
        let mut count = 0usize;
        for chat in &self.conversations {
            let members: HashSet<_> = chat.members.iter().copied().collect();
            if !conversations.insert(chat.id)
                || chat.title.len() > 200
                || !matches!(chat.kind.as_str(), "direct" | "group")
                || members.len() > 500
                || members.len() != chat.members.len()
                || !members.contains(&self.source_device)
                || members.iter().any(|id| !people.contains(id))
            {
                return Err(invalid());
            }
            count = count.checked_add(chat.messages.len()).ok_or_else(invalid)?;
            if count > 100_000 {
                return Err(invalid());
            }
            let mut messages = HashSet::new();
            for message in &chat.messages {
                if !messages.insert((message.sender, message.id))
                    || !members.contains(&message.sender)
                    || message.reply_to.as_ref().is_some_and(|target| {
                        target.message_id == message.id && target.sender == message.sender
                    })
                    || message.sent_at > 253_402_300_799
                    || message.text.len() > 8000
                    || message.reactions.len() > members.len()
                    || (message.text.trim().is_empty() && message.attachment.is_none())
                {
                    return Err(invalid());
                }
                let mut actors = HashSet::new();
                for reaction in &message.reactions {
                    if !members.contains(&reaction.sender)
                        || !actors.insert(reaction.sender)
                        || reaction.emoji.is_empty()
                        || reaction.emoji.len() > 64
                    {
                        return Err(invalid());
                    }
                }
                if let Some(file) = &message.attachment
                    && (file.name.is_empty()
                        || file.name.len() > 180
                        || file
                            .name
                            .chars()
                            .any(|c| c.is_control() || c == '/' || c == '\\')
                        || file.mime.len() > 100
                        || file.size > 32 * 1024
                        || file.data.len() > (32usize * 1024).div_ceil(3) * 4
                        || BASE64_STANDARD
                            .decode(&file.data)
                            .map_err(|_| invalid())?
                            .len()
                            != file.size)
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}

// Stop serialization at the format limit instead of allocating an unbounded JSON buffer.
fn encode_archive(archive: &Archive) -> Result<Zeroizing<Vec<u8>>> {
    struct Bounded(Zeroizing<Vec<u8>>);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_ARCHIVE_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("archive size limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded(Zeroizing::new(Vec::new()));
    serde_json::to_writer(&mut output, archive).map_err(|_| BackupError::Size)?;
    Ok(output.0)
}

/// Fresh random data key and salt for EVERY archive. Authenticated header binds
/// the version, derivation salt, and wrapped key to this complete ciphertext.
pub fn encrypt(archive: &Archive, recovery: &RecoveryCode) -> Result<Vec<u8>> {
    archive.validate()?;
    let plaintext = encode_archive(archive)?;
    let mut salt = [0; 32];
    OsRng.fill_bytes(&mut salt);
    let mut key = Zeroizing::new([0; 32]);
    OsRng.fill_bytes(key.as_mut());
    let mut bytes = [MAGIC, salt.as_slice()].concat();
    let wrapped = seal_record(key.as_slice(), &recovery.wrapping_key(&salt), &bytes)
        .map_err(|_| BackupError::Invalid)?;
    bytes.extend(wrapped);
    let payload = seal_record(&plaintext, &key, &bytes).map_err(|_| BackupError::Invalid)?;
    bytes.extend(payload);
    Ok(bytes)
}

pub fn decrypt(bytes: &[u8], recovery: &RecoveryCode) -> Result<Archive> {
    if bytes.len() > MAX_BACKUP_BYTES {
        return Err(BackupError::Size);
    }
    if bytes.len() < HEADER + 40 || !bytes.starts_with(MAGIC) {
        return Err(BackupError::Invalid);
    }
    let wrapping = recovery.wrapping_key(&bytes[MAGIC.len()..PREFIX]);
    let unwrapped = open_record(&bytes[PREFIX..HEADER], &wrapping, &bytes[..PREFIX])
        .map_err(|_| BackupError::Invalid)?;
    let key = Zeroizing::new(
        <[u8; 32]>::try_from(unwrapped.as_slice()).map_err(|_| BackupError::Invalid)?,
    );
    let plaintext =
        open_record(&bytes[HEADER..], &key, &bytes[..HEADER]).map_err(|_| BackupError::Invalid)?;
    let archive: Archive = serde_json::from_slice(&plaintext).map_err(|_| BackupError::Invalid)?;
    archive.validate()?;
    Ok(archive)
}

impl ClientStore {
    /// A single encrypted row: no writes to live MLS state/history/cursors/outbox.
    /// Restore is idempotent and never replaces an existing archive with the same ID.
    pub fn import_archive(&mut self, archive: &Archive) -> Result<bool> {
        archive.validate()?;
        let bytes = encode_archive(archive)?;
        let context = format!("recovered-history-v1:{}", archive.id);
        let sealed =
            seal_record(&bytes, &self.key, context.as_bytes()).map_err(|_| BackupError::Invalid)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| BackupError::Storage)?;
        let existing: Option<Vec<u8>> = tx
            .query_row(
                "SELECT sealed FROM recovered_archives WHERE id=?1",
                [archive.id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| BackupError::Storage)?;
        if let Some(existing) = existing {
            let previous = open_record(&existing, &self.key, context.as_bytes())
                .map_err(|_| BackupError::Invalid)?;
            if previous.as_slice() != bytes.as_slice() {
                return Err(BackupError::Invalid);
            }
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO recovered_archives(id,sealed) VALUES(?1,?2)",
            params![archive.id.to_string(), sealed],
        )
        .map_err(|_| BackupError::Storage)?;
        tx.commit().map_err(|_| BackupError::Storage)?;
        Ok(true)
    }
    pub fn recovered_archives(&self) -> Result<Vec<Archive>> {
        let mut statement = self
            .connection
            .prepare("SELECT id,sealed FROM recovered_archives ORDER BY rowid DESC")
            .map_err(|_| BackupError::Storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(|_| BackupError::Storage)?;
        let mut archives = Vec::new();
        for row in rows {
            let (id, sealed) = row.map_err(|_| BackupError::Storage)?;
            let bytes = open_record(
                &sealed,
                &self.key,
                format!("recovered-history-v1:{id}").as_bytes(),
            )
            .map_err(|_| BackupError::Invalid)?;
            let archive: Archive =
                serde_json::from_slice(&bytes).map_err(|_| BackupError::Invalid)?;
            archive.validate()?;
            if archive.id.to_string() != id {
                return Err(BackupError::Invalid);
            }
            archives.push(archive);
        }
        Ok(archives)
    }
    pub fn delete_archive(&mut self, id: MessageId) -> Result<()> {
        self.connection
            .execute(
                "DELETE FROM recovered_archives WHERE id=?1",
                [id.to_string()],
            )
            .map_err(|_| BackupError::Storage)?;
        Ok(())
    }
}
