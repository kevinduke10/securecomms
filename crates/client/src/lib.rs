//! Transactional client state and outbox, shared by future platform clients.
//!
//! A send advances the MLS ratchet and queues its exact ciphertext in the same
//! SQLite transaction, BEFORE a transport worker may access it. On restart,
//! retry the saved envelope instead of re-encrypting the plaintext.
use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_crypto::{CryptoError, Device};
use croaker_protocol::*;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::Zeroizing;

pub mod application;
pub mod backup;
mod history;
pub mod membership;
pub use history::{
    AppliedPage, HistoryMessage, ReceivePolicy, TransientMessage, WelcomeAuthorization,
};
pub use membership::{ApprovedAddition, PendingAdmission};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("local state unavailable or invalid")]
    Storage,
    #[error(
        "Attachment upload history is full (1,024 requests or 512 MiB of pending uploads). Finish pending uploads or use a new device workspace."
    )]
    MediaCapacity,
    #[error("this device state already exists; open it instead")]
    AlreadyExists,
}
type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingEvent {
    pub conversation_id: ConversationId,
    pub event: SubmitEvent,
}

pub struct ClientStore {
    connection: Connection,
    key: Zeroizing<[u8; 32]>,
}

fn connect(path: impl AsRef<Path>) -> Result<Connection> {
    #[cfg(unix)]
    if path.as_ref() != Path::new(":memory:") {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&path)
            .map_err(|_| ClientError::Storage)?;
    }
    let connection = Connection::open(path).map_err(|_| ClientError::Storage)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| ClientError::Storage)?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS state(id INTEGER PRIMARY KEY CHECK(id=1), sealed BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS outbox(position INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT NOT NULL UNIQUE, envelope TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS history(position INTEGER PRIMARY KEY AUTOINCREMENT, conversation TEXT NOT NULL, sender TEXT NOT NULL, message_id TEXT NOT NULL, sealed BLOB NOT NULL, UNIQUE(conversation,sender,message_id));
        CREATE TABLE IF NOT EXISTS stale_applications(conversation TEXT PRIMARY KEY, count INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS group_join_epochs(conversation TEXT PRIMARY KEY, epoch INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS applied_group_commits_v1(conversation TEXT NOT NULL,message_id TEXT NOT NULL,PRIMARY KEY(conversation,message_id));
        CREATE TABLE IF NOT EXISTS group_lifecycle(request_id TEXT PRIMARY KEY, conversation TEXT NOT NULL, envelope TEXT NOT NULL, acknowledged INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS group_admissions(request_id TEXT PRIMARY KEY, conversation TEXT NOT NULL, envelope TEXT NOT NULL, acknowledged INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS recovered_archives(id TEXT PRIMARY KEY, sealed BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS cursors(conversation TEXT PRIMARY KEY, cursor INTEGER NOT NULL);").map_err(|_| ClientError::Storage)?;
    Ok(connection)
}

impl ClientStore {
    /// `key` must be randomly generated and kept in platform secure storage.
    /// It is not a password and must never be derived by padding a password.
    pub fn create(
        path: impl AsRef<Path>,
        device: Device,
        key: Zeroizing<[u8; 32]>,
    ) -> Result<Self> {
        let connection = connect(path)?;
        let exists: bool = connection
            .query_row("SELECT EXISTS(SELECT 1 FROM state)", [], |r| r.get(0))
            .map_err(|_| ClientError::Storage)?;
        if exists {
            return Err(ClientError::AlreadyExists);
        }
        let sealed = device.seal(&key)?;
        connection
            .execute("INSERT INTO state(id,sealed) VALUES(1,?1)", [sealed])
            .map_err(|_| ClientError::Storage)?;
        Ok(Self { connection, key })
    }

    pub fn open(path: impl AsRef<Path>, key: Zeroizing<[u8; 32]>) -> Result<Self> {
        let connection = connect(path)?;
        let sealed: Vec<u8> = connection
            .query_row("SELECT sealed FROM state WHERE id=1", [], |r| r.get(0))
            .map_err(|_| ClientError::Storage)?;
        Device::open(&sealed, &key)?;
        Ok(Self { connection, key })
    }

    fn transaction<T>(
        &mut self,
        op: impl FnOnce(&Transaction<'_>, &mut Device) -> Result<T>,
    ) -> Result<T> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| ClientError::Storage)?;
        let sealed: Vec<u8> = tx
            .query_row("SELECT sealed FROM state WHERE id=1", [], |r| r.get(0))
            .map_err(|_| ClientError::Storage)?;
        let mut device = Device::open(&sealed, &self.key)?;
        let result = op(&tx, &mut device)?;
        let sealed = device.seal(&self.key)?;
        tx.execute("UPDATE state SET sealed=?1 WHERE id=1", [sealed])
            .map_err(|_| ClientError::Storage)?;
        tx.commit().map_err(|_| ClientError::Storage)?;
        Ok(result)
    }

    /// For local key-package generation and other operations without outgoing
    /// ciphertext. The callback MUST NOT send network requests or publish
    /// ciphertext: the transaction has not committed while it is running.
    pub fn update<T>(
        &mut self,
        op: impl FnOnce(&mut Device) -> std::result::Result<T, CryptoError>,
    ) -> Result<T> {
        self.transaction(|_, device| Ok(op(device)?))
    }

    pub fn queue_message(
        &mut self,
        conversation_id: ConversationId,
        plaintext: &[u8],
    ) -> Result<PendingEvent> {
        let key = Zeroizing::new(*self.key);
        self.transaction(|tx, device| {
            let ciphertext = device.encrypt(conversation_id, plaintext)?;
            let pending = PendingEvent {
                conversation_id,
                event: SubmitEvent {
                    protocol_version: API_VERSION,
                    message_id: MessageId::new(),
                    kind: EventKind::Application,
                    recipient: None,
                    ciphertext: BASE64_STANDARD.encode(ciphertext),
                },
            };
            tx.execute(
                "INSERT INTO outbox(message_id,envelope) VALUES(?1,?2)",
                params![
                    pending.event.message_id.to_string(),
                    serde_json::to_string(&pending).map_err(|_| ClientError::Storage)?
                ],
            )
            .map_err(|_| ClientError::Storage)?;
            history::save(
                tx,
                &HistoryMessage {
                    message_id: pending.event.message_id,
                    conversation_id,
                    sender: device.binding().device_id,
                    sent_at: clock_seconds()?,
                    plaintext: plaintext.to_vec(),
                    outgoing: true,
                    pending: true,
                },
                &key,
            )?;
            Ok(pending)
        })
    }

    pub fn queue_removal(
        &mut self,
        conversation_id: ConversationId,
        removed: DeviceId,
    ) -> Result<PendingEvent> {
        self.transaction(|tx, device| {
            let commit = device.remove_member(conversation_id, removed)?;
            let pending = PendingEvent {
                conversation_id,
                event: SubmitEvent {
                    protocol_version: API_VERSION,
                    message_id: MessageId::new(),
                    kind: EventKind::Commit,
                    recipient: None,
                    ciphertext: BASE64_STANDARD.encode(commit),
                },
            };
            tx.execute(
                "INSERT INTO outbox(message_id,envelope) VALUES(?1,?2)",
                params![
                    pending.event.message_id.to_string(),
                    serde_json::to_string(&pending).map_err(|_| ClientError::Storage)?
                ],
            )
            .map_err(|_| ClientError::Storage)?;
            Ok(pending)
        })
    }

    /// Publish short-lived application signals without retaining their plaintext
    /// in local chat history. Ratchet state and ciphertext still commit together.
    pub fn queue_transient(
        &mut self,
        conversation_id: ConversationId,
        plaintext: &[u8],
    ) -> Result<PendingEvent> {
        self.transaction(|tx, device| {
            let ciphertext = device.encrypt(conversation_id, plaintext)?;
            let pending = PendingEvent {
                conversation_id,
                event: SubmitEvent {
                    protocol_version: API_VERSION,
                    message_id: MessageId::new(),
                    kind: EventKind::Application,
                    recipient: None,
                    ciphertext: BASE64_STANDARD.encode(ciphertext),
                },
            };
            tx.execute(
                "INSERT INTO outbox(message_id,envelope) VALUES(?1,?2)",
                params![
                    pending.event.message_id.to_string(),
                    serde_json::to_string(&pending).map_err(|_| ClientError::Storage)?
                ],
            )
            .map_err(|_| ClientError::Storage)?;
            Ok(pending)
        })
    }

    /// Retry these exact envelopes until the authenticated relay acknowledges.
    pub fn pending(&self) -> Result<Vec<PendingEvent>> {
        let mut stmt = self
            .connection
            .prepare("SELECT envelope FROM outbox ORDER BY position LIMIT 32")
            .map_err(|_| ClientError::Storage)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| ClientError::Storage)?;
        rows.map(|row| {
            serde_json::from_str(&row.map_err(|_| ClientError::Storage)?)
                .map_err(|_| ClientError::Storage)
        })
        .collect()
    }

    pub fn acknowledge(&mut self, message: MessageId) -> Result<()> {
        self.connection
            .execute(
                "DELETE FROM outbox WHERE message_id=?1",
                [message.to_string()],
            )
            .map_err(|_| ClientError::Storage)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use croaker_crypto::Received;

    #[test]
    fn restart_retries_exact_ciphertext_without_rolling_back_the_ratchet() {
        let tenant = TenantId::new();
        let id = ConversationId::new();
        let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
        let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
        alice.create_conversation(id).unwrap();
        let welcome = alice
            .add_member(id, &bob.key_package().unwrap(), bob.binding())
            .unwrap();
        bob.join(id, &welcome.welcome, &[alice.binding().clone()])
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.sqlite");
        let key = [81; 32];
        let mut store = ClientStore::create(&path, alice, Zeroizing::new(key)).unwrap();
        let first = store.queue_message(id, b"private first message").unwrap();
        drop(store);
        assert!(ClientStore::open(&path, Zeroizing::new([80; 32])).is_err());
        let mut store = ClientStore::open(&path, Zeroizing::new(key)).unwrap();
        assert_eq!(store.pending().unwrap(), vec![first.clone()]);
        let bytes = BASE64_STANDARD.decode(&first.event.ciphertext).unwrap();
        assert!(
            matches!(bob.receive(id, &bytes).unwrap(), Received::Application { plaintext, .. } if plaintext == b"private first message")
        );
        let result = store.update(|device| {
            device.encrypt(id, b"aborted operation")?;
            Err::<(), _>(CryptoError::InvalidInput)
        });
        assert!(result.is_err());
        let second = store.queue_message(id, b"private second message").unwrap();
        let bytes = BASE64_STANDARD.decode(&second.event.ciphertext).unwrap();
        assert!(
            matches!(bob.receive(id, &bytes).unwrap(), Received::Application { plaintext, .. } if plaintext == b"private second message")
        );
        store.acknowledge(first.event.message_id).unwrap();
        assert_eq!(store.pending().unwrap(), vec![second]);
        drop(store);
        let file = std::fs::read(path).unwrap();
        assert!(
            !file
                .windows(b"private first message".len())
                .any(|bytes| bytes == b"private first message")
        );
    }
}

pub mod lifecycle;
pub mod media;

fn clock_seconds() -> Result<u64> {
    #[cfg(target_arch = "wasm32")]
    {
        Ok((js_sys::Date::now() / 1000.0) as u64)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .map_err(|_| ClientError::Storage)
    }
}

#[cfg(target_arch = "wasm32")]
impl ClientStore {
    pub fn browser_export(&self) -> Result<Vec<u8>> {
        self.connection
            .serialize("main")
            .map(|data| data.to_vec())
            .map_err(|_| ClientError::Storage)
    }
    pub fn browser_open(bytes: &[u8], key: Zeroizing<[u8; 32]>) -> Result<Self> {
        use rusqlite::{ffi, serialize::OwnedData};
        let mut connection = Connection::open_in_memory().map_err(|_| ClientError::Storage)?;
        let pointer = unsafe { ffi::sqlite3_malloc64(bytes.len() as u64) as *mut u8 };
        let pointer = std::ptr::NonNull::new(pointer).ok_or(ClientError::Storage)?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.as_ptr(), bytes.len());
        }
        let data = unsafe { OwnedData::from_raw_nonnull(pointer, bytes.len()) };
        connection
            .deserialize("main", data, false)
            .map_err(|_| ClientError::Storage)?;
        let sealed: Vec<u8> = connection
            .query_row("SELECT sealed FROM state WHERE id=1", [], |r| r.get(0))
            .map_err(|_| ClientError::Storage)?;
        Device::open(&sealed, &key)?;
        Ok(Self { connection, key })
    }
}
