pub mod api;
#[cfg(feature = "aws")]
pub mod dynamodb;
#[cfg(feature = "local")]
pub mod sqlite;

use async_trait::async_trait;
use croaker_protocol::*;
use sha2::{Digest, Sha256};

pub const RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;

pub async fn serve(listener: tokio::net::TcpListener, router: axum::Router) -> std::io::Result<()> {
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("authentication required")]
    Unauthorized,
    #[error("resource not found or access denied")]
    NotFound,
    #[error("conflicting request")]
    Conflict,
    #[error("invalid request: {0}")]
    Invalid(&'static str),
    #[error("storage unavailable")]
    Unavailable,
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn event_hash(event: &SubmitEvent) -> String {
    // Struct field order is fixed; unknown fields are rejected at the boundary.
    hex::encode(Sha256::digest(
        serde_json::to_vec(event).expect("serializable event"),
    ))
}

pub fn validate_conversation(
    principal: &Principal,
    input: &CreateConversation,
) -> Result<Vec<DeviceId>> {
    if principal.role == Role::Guest {
        return Err(StoreError::NotFound);
    }
    let mut devices = input.devices.clone();
    devices.sort();
    devices.dedup();
    if devices.len() != input.devices.len()
        || devices.len() < 2
        || devices.len() > MAX_CONVERSATION_DEVICES
        || !devices.contains(&principal.device_id)
    {
        return Err(StoreError::Invalid(
            "provide 2–500 distinct devices, including yourself",
        ));
    }
    Ok(devices)
}

pub fn validate_event(
    principal: &Principal,
    conversation: &Conversation,
    input: &SubmitEvent,
) -> Result<()> {
    input.validate().map_err(StoreError::Invalid)?;
    if !conversation.devices.contains(&principal.device_id) {
        return Err(StoreError::NotFound);
    }
    if input.kind != EventKind::Application && principal.device_id != conversation.owner {
        return Err(StoreError::NotFound);
    }
    if input
        .recipient
        .is_some_and(|id| id == principal.device_id || !conversation.devices.contains(&id))
    {
        return Err(StoreError::Invalid("invalid welcome recipient"));
    }
    Ok(())
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

pub fn page(events: Vec<StoredEvent>, cursor: u64, limit: usize, device: DeviceId) -> SyncPage {
    let has_more = events.len() > limit;
    let scanned: Vec<_> = events.into_iter().take(limit).collect();
    let next_cursor = scanned.last().map_or(cursor, |e| e.sequence);
    let timestamp = now();
    let events = scanned
        .into_iter()
        .filter(|e| {
            e.expires_at > timestamp
                && e.sender != device
                && e.event.recipient.is_none_or(|id| id == device)
        })
        .collect();
    SyncPage {
        events,
        next_cursor,
        has_more,
    }
}

/// Every operation rechecks active registration. Stores must enforce checks and
/// writes in a transaction, so revocation cannot race an accepted write.
#[async_trait]
pub trait Store: Send + Sync {
    async fn authenticate(&self, token: &str) -> Result<Principal>;
    /// Operator-only entry point, never exposed as an HTTP enrollment endpoint.
    async fn provision(&self, principal: &Principal, token: &str) -> Result<()>;
    async fn revoke(&self, actor: &Principal, device: DeviceId) -> Result<()>;
    async fn create(&self, actor: &Principal, input: CreateConversation) -> Result<Conversation>;
    async fn append(
        &self,
        actor: &Principal,
        conversation: ConversationId,
        input: SubmitEvent,
    ) -> Result<AcceptedEvent>;
    async fn sync(
        &self,
        actor: &Principal,
        conversation: ConversationId,
        cursor: u64,
        limit: usize,
    ) -> Result<SyncPage>;
}
