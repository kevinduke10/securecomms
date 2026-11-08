//! Explicit local group admission. Journals contain public membership decisions,
//! authenticated under the approving client's vault key; private MLS state stays
//! in ClientStore and the relay receives only the saved encrypted admission.
use super::*;
use croaker_client::membership::ApprovedAddition;
use croaker_crypto::{open_record, seal_record};
use croaker_protocol::membership::{AcceptedAdmission, GroupAdmission};

const JOURNAL_VERSION: u16 = 1;
const MAX_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalAdmission {
    conversation_id: ConversationId,
    request_id: MessageId,
    owner_profile: String,
    target_profile: String,
    expected_devices: Vec<DeviceId>,
    authority: DeviceBinding,
    added: DeviceBinding,
    package: Vec<u8>,
    admission: Option<GroupAdmission>,
    accepted: Option<AcceptedAdmission>,
    completed: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupJournal {
    version: u16,
    owner: DeviceId,
    records: Vec<LocalAdmission>,
}

impl Workspace {
    fn group_journal_path(&self, owner: &Profile) -> PathBuf {
        self.directory.join(format!(
            "group-memberships-{}.vault",
            owner.principal.device_id
        ))
    }
    fn group_journal_key(owner: &Profile) -> Result<Zeroizing<[u8; 32]>> {
        let bytes = Zeroizing::new(hex::decode(&owner.vault_key)?);
        Ok(Zeroizing::new(
            <[u8; 32]>::try_from(bytes.as_slice()).context("Invalid local vault key.")?,
        ))
    }
    fn read_group_journal(&self, owner: &Profile) -> Result<GroupJournal> {
        let path = self.group_journal_path(owner);
        if !path.exists() {
            return Ok(GroupJournal {
                version: JOURNAL_VERSION,
                owner: owner.principal.device_id,
                records: vec![],
            });
        }
        ensure!(
            std::fs::metadata(&path)?.len() <= MAX_JOURNAL_BYTES + 40,
            "Group membership journal exceeds its size limit."
        );
        let ciphertext = std::fs::read(path)?;
        let bytes = open_record(
            &ciphertext,
            &*Self::group_journal_key(owner)?,
            format!(
                "cirra-local-group-memberships-v1:{}",
                owner.principal.device_id
            )
            .as_bytes(),
        )?;
        let journal: GroupJournal = serde_json::from_slice(&bytes)?;
        ensure!(
            journal.version == JOURNAL_VERSION && journal.owner == owner.principal.device_id,
            "Invalid group membership journal."
        );
        ensure!(
            journal.records.len() <= 10_000,
            "Group membership journal has too many entries."
        );
        Ok(journal)
    }
    fn save_group_journal(&self, owner: &Profile, journal: &GroupJournal) -> Result<()> {
        let bytes = Zeroizing::new(serde_json::to_vec(journal)?);
        ensure!(
            bytes.len() as u64 <= MAX_JOURNAL_BYTES,
            "Group membership journal exceeds its size limit."
        );
        let sealed = seal_record(
            &bytes,
            &*Self::group_journal_key(owner)?,
            format!(
                "cirra-local-group-memberships-v1:{}",
                owner.principal.device_id
            )
            .as_bytes(),
        )?;
        write_private(&self.group_journal_path(owner), &sealed)?;
        #[cfg(unix)]
        std::fs::File::open(&self.directory)?.sync_all()?;
        Ok(())
    }

    /// Resume durable approvals before any message outbox is submitted. Completed
    /// journals remain as pinned approval evidence for members catching up later.
    pub(crate) async fn flush_group_admissions(&mut self, alias: &str) -> Result<()> {
        if !self.online.get(alias).copied().unwrap_or(false) {
            return Ok(());
        }
        let owner = self.profile(alias)?;
        let mut journal = self.read_group_journal(&owner)?;
        for index in 0..journal.records.len() {
            if journal.records[index].completed {
                continue;
            }
            let mut record = journal.records[index].clone();
            ensure!(
                record.owner_profile == alias && record.authority == owner.binding,
                "Group approval owner mismatch."
            );
            let target = self.profile(&record.target_profile)?;
            ensure!(
                target.binding == record.added
                    && target.principal.tenant_id == owner.principal.tenant_id,
                "Group approval target mismatch."
            );
            if record.admission.is_none() {
                let admission = self.clients.get_mut(alias).unwrap().queue_admission(
                    record.conversation_id,
                    record.request_id,
                    &record.expected_devices,
                    &record.package,
                    &record.added,
                )?;
                record.admission = Some(admission);
                journal.records[index] = record.clone();
                self.save_group_journal(&owner, &journal)?;
            }
            if record.accepted.is_none() {
                let actor = self.relay.authenticate(&owner.token).await?;
                // The exact saved request is retried after ambiguous responses.
                record.accepted = Some(
                    self.relay
                        .admit(
                            &actor,
                            record.conversation_id,
                            record.admission.as_ref().unwrap().clone(),
                        )
                        .await?,
                );
                journal.records[index] = record.clone();
                self.save_group_journal(&owner, &journal)?;
            }
            let accepted = record.accepted.as_ref().unwrap();
            let candidate = self
                .clients
                .get_mut(&record.target_profile)
                .context("New device store is unavailable.")?;
            // A prior attempt may have exposed the roster, then the candidate
            // atomically joined before the final journal completion was saved.
            // Its committed cursor proves that welcome processing already ran;
            // never reset it or reconstruct its live keys.
            if candidate.cursor(record.conversation_id)? < accepted.welcome_sequence {
                candidate.prepare_join(record.conversation_id, accepted.commit_sequence)?;
            }
            let chat = self
                .manifest
                .chats
                .iter_mut()
                .find(|chat| chat.id == record.conversation_id && chat.owner == alias)
                .context("Group approval conversation mismatch.")?;
            ensure!(
                chat.kind == "group",
                "Only group conversations support adding devices."
            );
            if !chat.members.contains(&record.target_profile) {
                chat.members.push(record.target_profile.clone());
                if let Err(error) = self.save() {
                    self.manifest
                        .chats
                        .iter_mut()
                        .find(|chat| chat.id == record.conversation_id)
                        .unwrap()
                        .members
                        .retain(|member| member != &record.target_profile);
                    return Err(error);
                }
            }
            self.clients
                .get_mut(alias)
                .unwrap()
                .acknowledge_admission(record.request_id)?;
            record.completed = true;
            journal.records[index] = record;
            self.save_group_journal(&owner, &journal)?;
        }
        Ok(())
    }
    /// Call at open, before any profile can receive a welcome or publish messages.
    pub(crate) async fn recover_group_memberships(&mut self) -> Result<()> {
        let aliases: Vec<_> = self
            .manifest
            .profiles
            .iter()
            .map(|profile| profile.alias.clone())
            .collect();
        for alias in aliases {
            self.flush_group_admissions(&alias).await?;
            self.flush_group_lifecycles(&alias).await?;
        }
        Ok(())
    }
    /// No relay-provided metadata can create these authorizations. Only saved,
    /// authenticated local owner decisions authorize their exact commit IDs.
    pub(crate) fn approved_group_additions(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<ApprovedAddition>> {
        let mut approvals = Vec::new();
        for owner in &self.manifest.profiles {
            for record in self.read_group_journal(owner)?.records {
                if record.conversation_id == conversation && record.accepted.is_some() {
                    let admission = record.admission.context("Missing saved group admission.")?;
                    approvals.push(ApprovedAddition {
                        commit_id: admission.commit.message_id,
                        authority: record.authority,
                        added: record.added,
                    });
                }
            }
        }
        Ok(approvals)
    }

    pub async fn group_members(
        &mut self,
        alias: &str,
        conversation: ConversationId,
    ) -> Result<serde_json::Value> {
        let chat = self.member_chat(alias, conversation)?;
        ensure!(
            chat.kind == "group",
            "Only group conversations support adding devices."
        );
        let owner = self.profile(alias)?;
        ensure!(
            self.relay.authenticate(&owner.token).await? == owner.principal,
            "Local device credential does not match its identity."
        );
        let mut candidates = Vec::new();
        let mut members = Vec::new();
        for profile in &self.manifest.profiles {
            if profile.principal.tenant_id != owner.principal.tenant_id {
                continue;
            }
            let value = serde_json::json!({"profile":profile.alias,"name":profile.name,"label":profile.title,"fingerprint":profile.binding.fingerprint(),"owner":profile.alias==chat.owner});
            if chat.members.contains(&profile.alias) {
                members.push(value);
            } else if chat.owner == alias
                && self.clients[&profile.alias].cursor(conversation)? == 0
                && self
                    .relay
                    .authenticate(&profile.token)
                    .await
                    .is_ok_and(|actor| actor == profile.principal)
            {
                candidates.push(value);
            }
        }
        Ok(
            serde_json::json!({"can_invite":chat.owner==alias && self.online.get(alias)==Some(&true),"members":members,"candidates":candidates}),
        )
    }

    pub async fn add_group_member(
        &mut self,
        alias: &str,
        conversation: ConversationId,
        request_id: MessageId,
        target_alias: &str,
        fingerprint: &str,
    ) -> Result<()> {
        let owner = self.profile(alias)?;
        let chat = self.member_chat(alias, conversation)?;
        ensure!(
            chat.kind == "group" && chat.owner == alias,
            "Only the conversation owner can add a device."
        );
        ensure!(
            self.online.get(alias) == Some(&true),
            "Reconnect before adding a device."
        );
        let target = self.profile(target_alias)?;
        ensure!(
            target.principal.tenant_id == owner.principal.tenant_id,
            "Device belongs to another business."
        );
        ensure!(
            fingerprint.len() <= 128
                && fingerprint
                    .split_whitespace()
                    .collect::<String>()
                    .to_lowercase()
                    == target.binding.fingerprint(),
            "Device fingerprint does not match."
        );
        ensure!(
            self.relay.authenticate(&owner.token).await? == owner.principal,
            "Local owner credential does not match its identity."
        );
        ensure!(
            self.relay.authenticate(&target.token).await? == target.principal,
            "Local device credential does not match its identity."
        );
        let mut journal = self.read_group_journal(&owner)?;
        if let Some(previous) = journal
            .records
            .iter()
            .find(|record| record.request_id == request_id)
        {
            ensure!(
                previous.conversation_id == conversation
                    && previous.target_profile == target_alias
                    && previous.added == target.binding,
                "Conflicting group invitation request."
            );
            return self.flush_group_admissions(alias).await;
        }
        ensure!(
            !chat.members.iter().any(|member| member == target_alias),
            "Device is already a group member."
        );
        // Sync only the approving account. Other participants' saved envelopes
        // remain unchanged and retain normal offline retry semantics.
        self.sync_profile(alias).await?;
        journal = self.read_group_journal(&owner)?;
        ensure!(
            journal.records.len() < 10_000,
            "Group membership journal has too many entries."
        );
        let chat = self.member_chat(alias, conversation)?;
        let expected_devices = chat
            .members
            .iter()
            .map(|member| {
                self.profile(member)
                    .map(|profile| profile.principal.device_id)
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            !self
                .clients
                .get_mut(target_alias)
                .context("Device store unavailable.")?
                .update(|device| Ok(device.has_conversation(conversation)))?,
            "A previously removed device cannot rejoin with old group state. Enroll a new device first."
        );
        let package = self
            .clients
            .get_mut(target_alias)
            .context("New device store is unavailable.")?
            .update(Device::key_package)?;
        journal.records.push(LocalAdmission {
            conversation_id: conversation,
            request_id,
            owner_profile: alias.into(),
            target_profile: target_alias.into(),
            expected_devices,
            authority: owner.binding.clone(),
            added: target.binding,
            package,
            admission: None,
            accepted: None,
            completed: false,
        });
        self.save_group_journal(&owner, &journal)?;
        self.flush_group_admissions(alias).await
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteInput {
    request_id: MessageId,
    target_profile: String,
    fingerprint: String,
}
async fn members(
    State(workspace): State<Shared>,
    RoutePath((alias, conversation)): RoutePath<(String, ConversationId)>,
) -> std::result::Result<Json<serde_json::Value>, LocalError> {
    Ok(Json(
        workspace
            .lock()
            .await
            .group_members(&alias, conversation)
            .await?,
    ))
}
async fn invite(
    State(workspace): State<Shared>,
    RoutePath((alias, conversation)): RoutePath<(String, ConversationId)>,
    Json(input): Json<InviteInput>,
) -> std::result::Result<StatusCode, LocalError> {
    workspace
        .lock()
        .await
        .add_group_member(
            &alias,
            conversation,
            input.request_id,
            &input.target_profile,
            &input.fingerprint,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) fn routes() -> Router<Shared> {
    Router::new().route(
        "/{profile}/conversations/{conversation}/members",
        get(members).post(invite),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn accepted_admission_recovery_never_resets_a_joined_client() {
        let directory = tempfile::tempdir().unwrap();
        let mut workspace = Workspace::open(directory.path().into()).await.unwrap();
        let initial = workspace.snapshot("alice").await.unwrap();
        let group: ConversationId = serde_json::from_value(
            initial["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|chat| chat["title"] == "Product notes")
                .unwrap()["id"]
                .clone(),
        )
        .unwrap();
        let fingerprint = workspace.profile("maya").unwrap().binding.fingerprint();
        let id = MessageId::new();
        workspace
            .add_group_member("alice", group, id, "maya", &fingerprint)
            .await
            .unwrap();
        let joined = workspace.snapshot("maya").await.unwrap();
        assert!(
            joined["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|chat| chat["id"] == group.to_string())
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let owner = workspace.profile("alice").unwrap();
        let mut journal = workspace.read_group_journal(&owner).unwrap();
        let original = serde_json::to_vec(&journal.records[0].admission).unwrap();
        // Model loss of the local receipt/completion after the relay transaction
        // succeeded and the recipient already authenticated its welcome.
        journal.records[0].accepted = None;
        journal.records[0].completed = false;
        workspace.save_group_journal(&owner, &journal).unwrap();
        drop(workspace);
        let mut workspace = Workspace::open(directory.path().into()).await.unwrap();
        workspace.recover_group_memberships().await.unwrap();
        let journal = workspace
            .read_group_journal(&workspace.profile("alice").unwrap())
            .unwrap();
        assert!(journal.records[0].completed);
        assert_eq!(
            serde_json::to_vec(&journal.records[0].admission).unwrap(),
            original
        );
        workspace
            .send(
                "alice",
                group,
                MessageId::new(),
                serde_json::from_value(serde_json::json!({"version":1,"text":"Message after recovered group admission","attachment":null})).unwrap(),
            )
            .await
            .unwrap();
        let received = workspace.snapshot("maya").await.unwrap();
        assert!(
            received["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|chat| chat["id"] == group.to_string())
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["text"] == "Message after recovered group admission")
        );
    }
}
