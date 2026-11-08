use super::*;
use croaker_crypto::{DeviceBinding, Received, open_record, seal_record};
use rusqlite::OptionalExtension;

#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    pub message_id: MessageId,
    pub conversation_id: ConversationId,
    pub sender: DeviceId,
    pub sent_at: u64,
    pub plaintext: Vec<u8>,
    pub outgoing: bool,
    #[serde(default)]
    pub pending: bool,
}

pub struct TransientMessage {
    pub sequence: u64,
    pub message: HistoryMessage,
}

/// Trusted local join intent, independent of relay metadata.
pub struct WelcomeAuthorization<'a> {
    pub authority: &'a DeviceBinding,
    pub members: &'a [DeviceId],
}
#[derive(Default)]
pub struct ReceivePolicy<'a> {
    pub additions: &'a [crate::membership::ApprovedAddition],
    pub lifecycles: &'a [crate::lifecycle::ApprovedLifecycle],
    pub welcome: Option<WelcomeAuthorization<'a>>,
}

pub struct AppliedPage {
    pub transient: Vec<TransientMessage>,
    pub skipped_stale_applications: usize,
}

fn context(conversation: ConversationId, sender: DeviceId, message: MessageId) -> Vec<u8> {
    format!("history:{conversation}:{sender}:{message}").into_bytes()
}

pub(super) fn save(tx: &Transaction<'_>, message: &HistoryMessage, key: &[u8; 32]) -> Result<()> {
    let plaintext = Zeroizing::new(serde_json::to_vec(message).map_err(|_| ClientError::Storage)?);
    let sealed = seal_record(
        &plaintext,
        key,
        &context(message.conversation_id, message.sender, message.message_id),
    )?;
    tx.execute(
        "INSERT INTO history(conversation,sender,message_id,sealed) VALUES(?1,?2,?3,?4)",
        params![
            message.conversation_id.to_string(),
            message.sender.to_string(),
            message.message_id.to_string(),
            sealed
        ],
    )
    .map_err(|_| ClientError::Storage)?;
    Ok(())
}

impl ClientStore {
    pub fn cursor(&self, conversation: ConversationId) -> Result<u64> {
        Ok(self
            .connection
            .query_row(
                "SELECT cursor FROM cursors WHERE conversation=?1",
                [conversation.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| ClientError::Storage)?
            .unwrap_or(0))
    }

    pub fn history(&self, conversation: ConversationId) -> Result<Vec<HistoryMessage>> {
        let mut stmt = self.connection.prepare("SELECT h.sender,h.message_id,h.sealed,EXISTS(SELECT 1 FROM outbox o WHERE o.message_id=h.message_id) FROM history h WHERE h.conversation=?1 ORDER BY h.position").map_err(|_| ClientError::Storage)?;
        let rows = stmt
            .query_map([conversation.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })
            .map_err(|_| ClientError::Storage)?;
        rows.map(|row| {
            let (sender, id, sealed, pending) = row.map_err(|_| ClientError::Storage)?;
            let sender: DeviceId = sender.parse().map_err(|_| ClientError::Storage)?;
            let id: MessageId = id.parse().map_err(|_| ClientError::Storage)?;
            let bytes = open_record(&sealed, &self.key, &context(conversation, sender, id))?;
            let mut message: HistoryMessage =
                serde_json::from_slice(&bytes).map_err(|_| ClientError::Storage)?;
            message.pending = message.outgoing && pending;
            Ok(message)
        })
        .collect()
    }

    /// Ratchet state, decrypted history (encrypted locally), and the receive
    /// cursor commit together. Any invalid envelope rolls back the whole page.
    pub fn apply_page(
        &mut self,
        conversation: ConversationId,
        expected_cursor: u64,
        page: &SyncPage,
        peers: &[DeviceBinding],
    ) -> Result<()> {
        self.apply_page_with_transient(conversation, expected_cursor, page, peers, |_| false)
            .map(|_| ())
    }

    /// Authenticate every application event and commit its ratchet/cursor, but
    /// return classified transient signals only in memory after the commit.
    pub fn apply_page_with_transient(
        &mut self,
        conversation: ConversationId,
        expected_cursor: u64,
        page: &SyncPage,
        peers: &[DeviceBinding],
        is_transient: fn(&[u8]) -> bool,
    ) -> Result<Vec<TransientMessage>> {
        self.apply_page_with_membership(
            conversation,
            expected_cursor,
            page,
            peers,
            is_transient,
            &[],
        )
    }

    /// The approval list must come from independently trusted local membership
    /// decisions, never be inferred from a relay-provided roster alone.
    pub fn apply_page_with_membership(
        &mut self,
        conversation: ConversationId,
        expected_cursor: u64,
        page: &SyncPage,
        peers: &[DeviceBinding],
        is_transient: fn(&[u8]) -> bool,
        approvals: &[crate::membership::ApprovedAddition],
    ) -> Result<Vec<TransientMessage>> {
        self.apply_page_with_membership_report(
            conversation,
            expected_cursor,
            page,
            peers,
            is_transient,
            approvals,
        )
        .map(|report| report.transient)
    }

    pub fn apply_page_with_membership_report(
        &mut self,
        conversation: ConversationId,
        expected_cursor: u64,
        page: &SyncPage,
        peers: &[DeviceBinding],
        is_transient: fn(&[u8]) -> bool,
        approvals: &[crate::membership::ApprovedAddition],
    ) -> Result<AppliedPage> {
        self.apply_page_with_lifecycle_report(
            conversation,
            expected_cursor,
            page,
            peers,
            is_transient,
            approvals,
            &[],
        )
    }
    // Keep the existing receive API while adding separately pinned lifecycle decisions.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_page_with_lifecycle_report(
        &mut self,
        conversation: ConversationId,
        expected_cursor: u64,
        page: &SyncPage,
        peers: &[DeviceBinding],
        is_transient: fn(&[u8]) -> bool,
        approvals: &[crate::membership::ApprovedAddition],
        lifecycles: &[crate::lifecycle::ApprovedLifecycle],
    ) -> Result<AppliedPage> {
        self.apply_page_with_policy_report(
            conversation,
            expected_cursor,
            page,
            peers,
            is_transient,
            ReceivePolicy {
                additions: approvals,
                lifecycles,
                welcome: None,
            },
        )
    }

    /// Commit authorization checks, MLS state, local history and cursor together.
    pub fn apply_page_with_policy_report(
        &mut self,
        conversation: ConversationId,
        expected_cursor: u64,
        page: &SyncPage,
        peers: &[DeviceBinding],
        is_transient: fn(&[u8]) -> bool,
        policy: ReceivePolicy<'_>,
    ) -> Result<AppliedPage> {
        let approvals = policy.additions;
        let lifecycles = policy.lifecycles;
        if page.events.is_empty()
            && page.next_cursor == expected_cursor
            && self.cursor(conversation)? == expected_cursor
        {
            return Ok(AppliedPage {
                transient: Vec::new(),
                skipped_stale_applications: 0,
            });
        }
        let key = Zeroizing::new(*self.key);
        self.transaction(|tx, device| {
            let current: u64 = tx.query_row("SELECT cursor FROM cursors WHERE conversation=?1", [conversation.to_string()], |r| r.get(0)).optional().map_err(|_| ClientError::Storage)?.unwrap_or(0);
            if current != expected_cursor || page.next_cursor < current { return Err(ClientError::Storage); }
            let mut sequence = current;
            let mut transient = Vec::new();
            let mut skipped_stale_applications=0usize;
            for stored in &page.events {
                if stored.sequence <= sequence || stored.sequence > page.next_cursor { return Err(ClientError::Storage); }
                sequence = stored.sequence;
                let bytes = BASE64_STANDARD.decode(&stored.event.ciphertext).map_err(|_| ClientError::Storage)?;
                match stored.event.kind {
                    EventKind::Welcome => {
                        if let Some(authorization)=&policy.welcome {
                            if stored.sender!=authorization.authority.device_id {return Err(CryptoError::IdentityMismatch.into());}
                            device.join_authorized(conversation,&bytes,peers,authorization.authority,authorization.members)?;
                        } else {
                            device.join(conversation, &bytes, peers)?;
                        }
                        tx.execute("INSERT INTO group_join_epochs(conversation,epoch) VALUES(?1,?2)",params![conversation.to_string(),device.conversation_epoch(conversation)?]).map_err(|_|ClientError::Storage)?;
                    },
                    EventKind::Application => {
                        let known: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM history WHERE conversation=?1 AND sender=?2 AND message_id=?3)", params![conversation.to_string(), stored.sender.to_string(), stored.event.message_id.to_string()], |r| r.get(0)).map_err(|_| ClientError::Storage)?;
                        if known { continue; }
                        let join_epoch:u64=tx.query_row("SELECT epoch FROM group_join_epochs WHERE conversation=?1",[conversation.to_string()],|row|row.get(0)).optional().map_err(|_|ClientError::Storage)?.unwrap_or(0);
                        if device.stale_application(conversation,&bytes,join_epoch)? {skipped_stale_applications+=1;continue;}
                        let Received::Application { sender, plaintext } = device.receive(conversation, &bytes)?;
                        if sender != stored.sender { return Err(CryptoError::IdentityMismatch.into()); }
                        let message = HistoryMessage { message_id: stored.event.message_id, conversation_id: conversation, sender, sent_at: stored.submitted_at, plaintext, outgoing: false, pending: false };
                        if is_transient(&message.plaintext) { transient.push(TransientMessage { sequence: stored.sequence, message }); }
                        else { save(tx, &message, &key)?; }
                    }
                    EventKind::Commit => {
                        if let Some(approval)=lifecycles.iter().find(|approval|approval.context.request_id==stored.event.message_id) {
                            if stored.sender!=approval.authority.device_id || approval.context.conversation_id!=conversation {return Err(CryptoError::IdentityMismatch.into());}
                            device.apply_lifecycle(&approval.context,&bytes,&approval.authority,&approval.target)?;
                        } else {
                            let approval=approvals.iter().find(|approval|approval.commit_id==stored.event.message_id).ok_or(CryptoError::Mls("membership changes require explicit approval"))?;
                            if stored.sender!=approval.authority.device_id {return Err(CryptoError::IdentityMismatch.into());}
                            device.apply_addition(conversation,&bytes,&approval.authority,&approval.added)?;
                        }
                        tx.execute("INSERT OR IGNORE INTO applied_group_commits_v1(conversation,message_id) VALUES(?1,?2)",params![conversation.to_string(),stored.event.message_id.to_string()]).map_err(|_|ClientError::Storage)?;
                    },
                }
            }
            if skipped_stale_applications>0 {
                tx.execute("INSERT INTO stale_applications(conversation,count) VALUES(?1,?2) ON CONFLICT(conversation) DO UPDATE SET count=count+excluded.count",params![conversation.to_string(),skipped_stale_applications as u64]).map_err(|_|ClientError::Storage)?;
            }
            tx.execute("INSERT INTO cursors(conversation,cursor) VALUES(?1,?2) ON CONFLICT(conversation) DO UPDATE SET cursor=excluded.cursor", params![conversation.to_string(), page.next_cursor]).map_err(|_| ClientError::Storage)?;
            Ok(AppliedPage{transient,skipped_stale_applications})
        })
    }

    /// Initialize a new group and queue all addressed welcomes atomically.
    pub fn create_group(
        &mut self,
        conversation: ConversationId,
        packages: &[(Vec<u8>, DeviceBinding)],
    ) -> Result<()> {
        self.transaction(|tx, device| {
            device.create_conversation(conversation)?;
            let members: Vec<_> = packages
                .iter()
                .map(|(bytes, binding)| (bytes.as_slice(), binding))
                .collect();
            let added = device.add_members(conversation, &members)?;
            for (_, binding) in packages {
                let pending = PendingEvent {
                    conversation_id: conversation,
                    event: SubmitEvent {
                        protocol_version: API_VERSION,
                        message_id: MessageId::new(),
                        kind: EventKind::Welcome,
                        recipient: Some(binding.device_id),
                        ciphertext: BASE64_STANDARD.encode(&added.welcome),
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
            }
            Ok(())
        })
    }

    /// Local HTTP retries supply a stable ID. Once accepted locally, the same
    /// ID/content does not encrypt again, even after the relay acknowledged it.
    pub fn queue_message_once(
        &mut self,
        conversation: ConversationId,
        id: MessageId,
        plaintext: &[u8],
    ) -> Result<bool> {
        let key = Zeroizing::new(*self.key);
        self.transaction(|tx, device| {
            let sender = device.binding().device_id;
            let existing: Option<Vec<u8>> = tx.query_row("SELECT sealed FROM history WHERE conversation=?1 AND sender=?2 AND message_id=?3", params![conversation.to_string(), sender.to_string(), id.to_string()], |r| r.get(0)).optional().map_err(|_| ClientError::Storage)?;
            if let Some(existing) = existing {
                let bytes = open_record(&existing, &key, &context(conversation,sender,id))?;
                let previous: HistoryMessage = serde_json::from_slice(&bytes).map_err(|_| ClientError::Storage)?;
                if previous.plaintext != plaintext { return Err(ClientError::AlreadyExists); }
                return Ok(false);
            }
            let ciphertext = device.encrypt(conversation, plaintext)?;
            let pending = PendingEvent { conversation_id: conversation, event: SubmitEvent { protocol_version: API_VERSION, message_id: id, kind: EventKind::Application, recipient: None, ciphertext: BASE64_STANDARD.encode(ciphertext) } };
            tx.execute("INSERT INTO outbox(message_id,envelope) VALUES(?1,?2)", params![id.to_string(), serde_json::to_string(&pending).map_err(|_| ClientError::Storage)?]).map_err(|_| ClientError::Storage)?;
            save(tx, &HistoryMessage { message_id: id, conversation_id: conversation, sender, sent_at: crate::clock_seconds()?, plaintext: plaintext.to_vec(), outgoing: true, pending: true }, &key)?;
            Ok(true)
        })
    }
}
