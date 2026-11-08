use super::*;
use croaker_crypto::DeviceBinding;
use croaker_protocol::membership::GroupAdmission;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingAdmission {
    pub conversation_id: ConversationId,
    pub input: GroupAdmission,
}
#[derive(Serialize, Deserialize)]
struct Saved {
    pending: PendingAdmission,
    added: DeviceBinding,
    package_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovedAddition {
    pub commit_id: MessageId,
    pub authority: DeviceBinding,
    pub added: DeviceBinding,
}
impl ClientStore {
    pub fn stale_application_count(&self, conversation: ConversationId) -> Result<u64> {
        self.connection
            .query_row(
                "SELECT count FROM stale_applications WHERE conversation=?1",
                [conversation.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map(|count| count.unwrap_or(0))
            .map_err(|_| ClientError::Storage)
    }

    /// Advance MLS and save BOTH ciphertexts atomically. Never regenerate on retry.
    pub fn queue_admission(
        &mut self,
        conversation: ConversationId,
        request_id: MessageId,
        expected_devices: &[DeviceId],
        package: &[u8],
        added: &DeviceBinding,
    ) -> Result<GroupAdmission> {
        let package_hash = hex::encode(Sha256::digest(package));
        self.transaction(|tx,device| {
            let previous:Option<String>=tx.query_row("SELECT envelope FROM group_admissions WHERE request_id=?1",[request_id.to_string()],|r|r.get(0)).optional().map_err(|_|ClientError::Storage)?;
            if let Some(previous)=previous {
                let saved:Saved=serde_json::from_str(&previous).map_err(|_|ClientError::Storage)?;
                if saved.pending.conversation_id!=conversation || saved.pending.input.expected_devices!=expected_devices || saved.added!=*added || saved.package_hash!=package_hash {return Err(ClientError::AlreadyExists);}
                return Ok(saved.pending.input);
            }
            let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE json_extract(envelope,'$.conversation_id')=?1) OR EXISTS(SELECT 1 FROM group_admissions WHERE conversation=?1 AND acknowledged=0) OR EXISTS(SELECT 1 FROM group_lifecycle WHERE conversation=?1 AND acknowledged=0)",[conversation.to_string()],|r|r.get(0)).map_err(|_|ClientError::Storage)?;
            if pending {return Err(ClientError::AlreadyExists);}
            let mut expected=expected_devices.to_vec();expected.sort();
            if device.conversation_devices(conversation)?!=expected {return Err(CryptoError::IdentityMismatch.into());}
            let member=device.add_member(conversation,package,added)?;
            let input=GroupAdmission{request_id,expected_devices:expected_devices.to_vec(),added_device:added.device_id,
                commit:SubmitEvent{protocol_version:API_VERSION,message_id:request_id,kind:EventKind::Commit,recipient:None,ciphertext:BASE64_STANDARD.encode(member.commit)},
                welcome:SubmitEvent{protocol_version:API_VERSION,message_id:MessageId::new(),kind:EventKind::Welcome,recipient:Some(added.device_id),ciphertext:BASE64_STANDARD.encode(member.welcome)}};
            input.validate().map_err(|_|CryptoError::InvalidInput)?;
            let saved=Saved{pending:PendingAdmission{conversation_id:conversation,input:input.clone()},added:added.clone(),package_hash};
            tx.execute("INSERT INTO group_admissions(request_id,conversation,envelope) VALUES(?1,?2,?3)",params![request_id.to_string(),conversation.to_string(),serde_json::to_string(&saved).map_err(|_|ClientError::Storage)?]).map_err(|_|ClientError::Storage)?;
            Ok(input)
        })
    }
    pub fn pending_admissions(&self) -> Result<Vec<PendingAdmission>> {
        let mut statement = self
            .connection
            .prepare("SELECT envelope FROM group_admissions WHERE acknowledged=0 ORDER BY rowid")
            .map_err(|_| ClientError::Storage)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| ClientError::Storage)?;
        rows.map(|row| {
            serde_json::from_str::<Saved>(&row.map_err(|_| ClientError::Storage)?)
                .map(|saved| saved.pending)
                .map_err(|_| ClientError::Storage)
        })
        .collect()
    }
    pub fn acknowledge_admission(&mut self, id: MessageId) -> Result<()> {
        self.connection
            .execute(
                "UPDATE group_admissions SET acknowledged=1 WHERE request_id=?1",
                [id.to_string()],
            )
            .map_err(|_| ClientError::Storage)?;
        Ok(())
    }
    /// Establish a new device's delivery floor. It must not process pre-join epochs.
    pub fn prepare_join(
        &mut self,
        conversation: ConversationId,
        commit_sequence: u64,
    ) -> Result<()> {
        self.transaction(|tx, device| {
            let current: Option<u64> = tx
                .query_row(
                    "SELECT cursor FROM cursors WHERE conversation=?1",
                    [conversation.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| ClientError::Storage)?;
            if device.has_conversation(conversation)
                || current.is_some_and(|value| value != commit_sequence)
            {
                return Err(ClientError::AlreadyExists);
            }
            tx.execute(
                "INSERT OR IGNORE INTO cursors(conversation,cursor) VALUES(?1,?2)",
                params![conversation.to_string(), commit_sequence],
            )
            .map_err(|_| ClientError::Storage)?;
            Ok(())
        })
    }
}
