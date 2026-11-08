use super::*;
use croaker_crypto::DeviceBinding;
use croaker_protocol::lifecycle::*;
use rusqlite::OptionalExtension;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovedLifecycle {
    pub context: LifecycleContext,
    pub authority: DeviceBinding,
    pub target: DeviceBinding,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingLifecycle {
    pub conversation_id: ConversationId,
    pub input: GroupLifecycle,
}
#[derive(Serialize, Deserialize)]
struct Saved {
    pending: PendingLifecycle,
    target: DeviceBinding,
}
impl ClientStore {
    pub fn group_commit_applied(
        &self,
        conversation: ConversationId,
        id: MessageId,
    ) -> Result<bool> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM applied_group_commits_v1 WHERE conversation=?1 AND message_id=?2)",
                params![conversation.to_string(),id.to_string()],
                |r| r.get(0),
            )
            .map_err(|_| ClientError::Storage)
    }
    pub fn queue_lifecycle(
        &mut self,
        context: &LifecycleContext,
        target: &DeviceBinding,
    ) -> Result<GroupLifecycle> {
        self.transaction(|tx, device| {
            let previous: Option<String> = tx.query_row(
                "SELECT envelope FROM group_lifecycle WHERE request_id=?1",
                [context.request_id.to_string()], |r| r.get(0),
            ).optional().map_err(|_| ClientError::Storage)?;
            if let Some(previous) = previous {
                let saved: Saved = serde_json::from_str(&previous).map_err(|_| ClientError::Storage)?;
                if saved.pending.input.context != *context || saved.target != *target {
                    return Err(ClientError::AlreadyExists);
                }
                return Ok(saved.pending.input);
            }
            let pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM outbox WHERE json_extract(envelope,'$.conversation_id')=?1)
                 OR EXISTS(SELECT 1 FROM group_admissions WHERE conversation=?1 AND acknowledged=0)
                 OR EXISTS(SELECT 1 FROM group_lifecycle WHERE conversation=?1 AND acknowledged=0)",
                [context.conversation_id.to_string()], |r| r.get(0),
            ).map_err(|_| ClientError::Storage)?;
            if pending { return Err(ClientError::AlreadyExists); }
            let bytes = device.commit_lifecycle(context, target)?;
            let input = GroupLifecycle {
                context: context.clone(),
                commit: SubmitEvent {
                    protocol_version: API_VERSION,
                    message_id: context.request_id,
                    kind: EventKind::Commit,
                    recipient: None,
                    ciphertext: BASE64_STANDARD.encode(bytes),
                },
            };
            input.validate().map_err(|_| CryptoError::InvalidInput)?;
            let saved = Saved {
                pending: PendingLifecycle { conversation_id: context.conversation_id, input: input.clone() },
                target: target.clone(),
            };
            tx.execute(
                "INSERT INTO group_lifecycle(request_id,conversation,envelope) VALUES(?1,?2,?3)",
                params![context.request_id.to_string(),context.conversation_id.to_string(),
                    serde_json::to_string(&saved).map_err(|_| ClientError::Storage)?],
            ).map_err(|_| ClientError::Storage)?;
            Ok(input)
        })
    }

    pub fn pending_lifecycles(&self) -> Result<Vec<PendingLifecycle>> {
        let mut statement = self
            .connection
            .prepare("SELECT envelope FROM group_lifecycle WHERE acknowledged=0 ORDER BY rowid")
            .map_err(|_| ClientError::Storage)?;
        let rows = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| ClientError::Storage)?;
        rows.map(|r| {
            serde_json::from_str::<Saved>(&r.map_err(|_| ClientError::Storage)?)
                .map(|s| s.pending)
                .map_err(|_| ClientError::Storage)
        })
        .collect()
    }
    pub fn acknowledge_lifecycle(&mut self, id: MessageId) -> Result<()> {
        self.connection
            .execute(
                "UPDATE group_lifecycle SET acknowledged=1 WHERE request_id=?1",
                [id.to_string()],
            )
            .map_err(|_| ClientError::Storage)?;
        Ok(())
    }
}
