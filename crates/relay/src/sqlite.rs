use crate::*;
use async_trait::async_trait;
use croaker_protocol::*;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

/// Local durable adapter. Blocking SQLite work runs outside Tokio's executor.
#[derive(Clone)]
pub struct SqliteStore(Arc<Mutex<Connection>>);

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        #[cfg(unix)]
        if path.as_ref() != Path::new(":memory:") {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&path)
                .map_err(|_| StoreError::Unavailable)?;
        }
        let connection = Connection::open(path).map_err(|_| StoreError::Unavailable)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| StoreError::Unavailable)?;
        connection
            .execute_batch(include_str!("schema.sql"))
            .map_err(|_| StoreError::Unavailable)?;
        Ok(Self(Arc::new(Mutex::new(connection))))
    }

    async fn transaction<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let connection = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = connection.lock().map_err(|_| StoreError::Unavailable)?;
            let tx = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| StoreError::Unavailable)?;
            // Preserve deduplication receipts while scrubbing expired payloads.
            tx.execute(
                "UPDATE events SET payload=NULL WHERE expires_at<=?1 AND payload IS NOT NULL",
                [now()],
            )
            .map_err(|_| StoreError::Unavailable)?;
            let result = f(&tx)?;
            tx.commit().map_err(|_| StoreError::Unavailable)?;
            Ok(result)
        })
        .await
        .map_err(|_| StoreError::Unavailable)?
    }
}

fn active(tx: &Transaction<'_>, actor: &Principal) -> Result<()> {
    let json: Option<String> = tx
        .query_row(
            "SELECT principal FROM devices WHERE tenant=?1 AND device=?2 AND revoked=0",
            params![actor.tenant_id.to_string(), actor.device_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| StoreError::Unavailable)?;
    let stored: Principal = serde_json::from_str(&json.ok_or(StoreError::Unauthorized)?)
        .map_err(|_| StoreError::Unavailable)?;
    if &stored != actor {
        return Err(StoreError::Unauthorized);
    }
    Ok(())
}

fn conversation(
    tx: &Transaction<'_>,
    actor: &Principal,
    id: ConversationId,
) -> Result<(Conversation, u64)> {
    active(tx, actor)?;
    let row: Option<(String, u64)> = tx
        .query_row(
            "SELECT metadata, sequence FROM conversations WHERE tenant=?1 AND id=?2",
            params![actor.tenant_id.to_string(), id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| StoreError::Unavailable)?;
    let (json, sequence) = row.ok_or(StoreError::NotFound)?;
    let conversation: Conversation =
        serde_json::from_str(&json).map_err(|_| StoreError::Unavailable)?;
    if !conversation.devices.contains(&actor.device_id) {
        return Err(StoreError::NotFound);
    }
    Ok((conversation, sequence))
}

#[async_trait]
impl Store for SqliteStore {
    async fn authenticate(&self, token: &str) -> Result<Principal> {
        let hash = token_hash(token);
        self.transaction(move |tx| {
            let json: Option<String> = tx
                .query_row(
                    "SELECT principal FROM devices WHERE token_hash=?1 AND revoked=0",
                    [hash],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| StoreError::Unavailable)?;
            serde_json::from_str(&json.ok_or(StoreError::Unauthorized)?)
                .map_err(|_| StoreError::Unavailable)
        })
        .await
    }

    async fn provision(&self, principal: &Principal, token: &str) -> Result<()> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StoreError::Invalid(
                "use a random 32-byte hexadecimal token",
            ));
        }
        let hash = token_hash(token);
        let principal = principal.clone();
        self.transaction(move |tx| {
            tx.execute("INSERT INTO devices(tenant,device,token_hash,principal,revoked) VALUES(?1,?2,?3,?4,0)", params![principal.tenant_id.to_string(), principal.device_id.to_string(), hash, serde_json::to_string(&principal).map_err(|_| StoreError::Unavailable)?]).map_err(|e| match e {
                rusqlite::Error::SqliteFailure(error, _) if error.code == rusqlite::ErrorCode::ConstraintViolation => StoreError::Conflict,
                _ => StoreError::Unavailable,
            })?;
            Ok(())
        }).await
    }

    async fn revoke(&self, actor: &Principal, device: DeviceId) -> Result<()> {
        let actor = actor.clone();
        self.transaction(move |tx| {
            active(tx, &actor)?;
            if actor.role != Role::Admin && actor.device_id != device {
                return Err(StoreError::NotFound);
            }
            let changed = tx
                .execute(
                    "UPDATE devices SET revoked=1 WHERE tenant=?1 AND device=?2",
                    params![actor.tenant_id.to_string(), device.to_string()],
                )
                .map_err(|_| StoreError::Unavailable)?;
            if changed == 0 {
                return Err(StoreError::NotFound);
            }
            Ok(())
        })
        .await
    }

    async fn create(&self, actor: &Principal, input: CreateConversation) -> Result<Conversation> {
        let devices = validate_conversation(actor, &input)?;
        let actor = actor.clone();
        self.transaction(move |tx| {
            active(tx, &actor)?;
            for device in &devices {
                let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE tenant=?1 AND device=?2 AND revoked=0)", params![actor.tenant_id.to_string(), device.to_string()], |r| r.get(0)).map_err(|_| StoreError::Unavailable)?;
                if !exists { return Err(StoreError::NotFound); }
            }
            let result = Conversation { id: input.id, tenant_id: actor.tenant_id, owner: actor.device_id, devices };
            let json = serde_json::to_string(&result).map_err(|_| StoreError::Unavailable)?;
            let inserted = tx.execute("INSERT OR IGNORE INTO conversations(tenant,id,metadata,sequence) VALUES(?1,?2,?3,0)", params![actor.tenant_id.to_string(), input.id.to_string(), json]).map_err(|_| StoreError::Unavailable)?;
            if inserted == 0 {
                let (existing, _) = conversation(tx, &actor, input.id)?;
                if existing != result { return Err(StoreError::Conflict); }
            }
            Ok(result)
        }).await
    }

    async fn append(
        &self,
        actor: &Principal,
        id: ConversationId,
        input: SubmitEvent,
    ) -> Result<AcceptedEvent> {
        input.validate().map_err(StoreError::Invalid)?;
        let actor = actor.clone();
        self.transaction(move |tx| {
            let (group, current) = conversation(tx, &actor, id)?;
            validate_event(&actor, &group, &input)?;
            let previous: Option<(u64, String)> = tx.query_row("SELECT sequence, fingerprint FROM events WHERE tenant=?1 AND conversation=?2 AND sender=?3 AND message_id=?4", params![actor.tenant_id.to_string(), id.to_string(), actor.device_id.to_string(), input.message_id.to_string()], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(|_| StoreError::Unavailable)?;
            let fingerprint = event_hash(&input);
            if let Some((sequence, hash)) = previous {
                return if hash == fingerprint { Ok(AcceptedEvent { sequence, duplicate: true }) } else { Err(StoreError::Conflict) };
            }
            let timestamp = now();
            let event = StoredEvent { sequence: current + 1, sender: actor.device_id, submitted_at: timestamp, expires_at: timestamp + RETENTION_SECONDS, event: input };
            tx.execute("INSERT INTO events(tenant,conversation,sequence,sender,message_id,fingerprint,payload,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![actor.tenant_id.to_string(), id.to_string(), event.sequence, actor.device_id.to_string(), event.event.message_id.to_string(), fingerprint, serde_json::to_string(&event).map_err(|_| StoreError::Unavailable)?, event.expires_at]).map_err(|_| StoreError::Unavailable)?;
            tx.execute("UPDATE conversations SET sequence=?3 WHERE tenant=?1 AND id=?2", params![actor.tenant_id.to_string(), id.to_string(), event.sequence]).map_err(|_| StoreError::Unavailable)?;
            Ok(AcceptedEvent { sequence: event.sequence, duplicate: false })
        }).await
    }

    async fn sync(
        &self,
        actor: &Principal,
        id: ConversationId,
        cursor: u64,
        limit: usize,
    ) -> Result<SyncPage> {
        if limit == 0 || limit > MAX_SYNC_PAGE {
            return Err(StoreError::Invalid("page size must be 1–32"));
        }
        let actor = actor.clone();
        self.transaction(move |tx| {
            let (_, latest) = conversation(tx, &actor, id)?;
            if cursor > latest { return Err(StoreError::Invalid("cursor is ahead of conversation")); }
            let mut stmt = tx.prepare("SELECT payload FROM events WHERE tenant=?1 AND conversation=?2 AND sequence>?3 AND payload IS NOT NULL ORDER BY sequence LIMIT ?4").map_err(|_| StoreError::Unavailable)?;
            let rows = stmt.query_map(params![actor.tenant_id.to_string(), id.to_string(), cursor, limit + 1], |r| r.get::<_, String>(0)).map_err(|_| StoreError::Unavailable)?;
            let mut events = Vec::new();
            for row in rows { events.push(serde_json::from_str(&row.map_err(|_| StoreError::Unavailable)?).map_err(|_| StoreError::Unavailable)?); }
            let mut result = page(events, cursor, limit, actor.device_id);
            if !result.has_more { result.next_cursor = result.next_cursor.max(latest); }
            Ok(result)
        }).await
    }
}
