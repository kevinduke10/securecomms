//! Authenticated local decision journal; relay metadata cannot authorize a commit.
use super::*;
use croaker_client::lifecycle::ApprovedLifecycle;
use croaker_crypto::{open_record, seal_record};
use croaker_protocol::lifecycle::*;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    owner_profile: String,
    target_profile: String,
    approval: ApprovedLifecycle,
    input: Option<GroupLifecycle>,
    accepted: Option<AcceptedLifecycle>,
    completed: bool,
}
impl Workspace {
    fn lifecycle_path(&self, owner: &Profile) -> PathBuf {
        self.directory.join(format!(
            "group-lifecycle-{}.vault",
            owner.principal.device_id
        ))
    }
    fn lifecycle_key(owner: &Profile) -> Result<Zeroizing<[u8; 32]>> {
        let bytes = Zeroizing::new(hex::decode(&owner.vault_key)?);
        Ok(Zeroizing::new(
            bytes.as_slice().try_into().context("Invalid vault key.")?,
        ))
    }
    fn read_lifecycle(&self, owner: &Profile) -> Result<Vec<Record>> {
        let path = self.lifecycle_path(owner);
        if !path.exists() {
            return Ok(vec![]);
        }
        ensure!(
            std::fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
            "Group lifecycle journal exceeds its size limit."
        );
        let bytes = open_record(
            &std::fs::read(path)?,
            &*Self::lifecycle_key(owner)?,
            format!("cirra-local-lifecycle-v1:{}", owner.principal.device_id).as_bytes(),
        )?;
        let records: Vec<Record> = serde_json::from_slice(&bytes)?;
        ensure!(
            records.len() <= 10_000,
            "Group lifecycle journal has too many entries."
        );
        Ok(records)
    }
    fn save_lifecycle(&self, owner: &Profile, records: &[Record]) -> Result<()> {
        let bytes = Zeroizing::new(serde_json::to_vec(records)?);
        ensure!(
            bytes.len() <= 16 * 1024 * 1024 - 40,
            "Group lifecycle journal exceeds its size limit."
        );
        let encrypted = seal_record(
            &bytes,
            &*Self::lifecycle_key(owner)?,
            format!("cirra-local-lifecycle-v1:{}", owner.principal.device_id).as_bytes(),
        )?;
        write_private(&self.lifecycle_path(owner), &encrypted)?;
        #[cfg(unix)]
        std::fs::File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
    pub(crate) fn approved_group_lifecycles(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<ApprovedLifecycle>> {
        let mut out = vec![];
        for owner in &self.manifest.profiles {
            for record in self.read_lifecycle(owner)? {
                if record.approval.context.conversation_id == conversation
                    && record.accepted.is_some()
                {
                    out.push(record.approval);
                }
            }
        }
        Ok(out)
    }
    pub(crate) async fn flush_group_lifecycles(&mut self, alias: &str) -> Result<()> {
        if self.online.get(alias) != Some(&true) {
            return Ok(());
        }
        let owner = self.profile(alias)?;
        let mut records = self.read_lifecycle(&owner)?;
        for index in 0..records.len() {
            let mut record = records[index].clone();
            if record.completed {
                continue;
            }
            ensure!(
                record.owner_profile == alias && record.approval.authority == owner.binding,
                "Group lifecycle owner mismatch."
            );
            let target = self.profile(&record.target_profile)?;
            ensure!(
                target.binding == record.approval.target,
                "Group lifecycle target mismatch."
            );
            if record.input.is_none() {
                record.input = Some(
                    self.clients
                        .get_mut(alias)
                        .unwrap()
                        .queue_lifecycle(&record.approval.context, &target.binding)?,
                );
                records[index] = record.clone();
                self.save_lifecycle(&owner, &records)?;
            }
            if record.accepted.is_none() {
                let actor = self.relay.authenticate(&owner.token).await?;
                record.accepted = Some(
                    self.relay
                        .lifecycle(
                            &actor,
                            record.approval.context.conversation_id,
                            record.input.clone().unwrap(),
                        )
                        .await?,
                );
                records[index] = record.clone();
                self.save_lifecycle(&owner, &records)?;
            }
            let chat = self
                .manifest
                .chats
                .iter_mut()
                .find(|c| c.id == record.approval.context.conversation_id)
                .context("Group no longer exists.")?;
            let previous_chat = chat.clone();
            match record.approval.context.action {
                LifecycleAction::Transfer { .. } => chat.owner = record.target_profile.clone(),
                LifecycleAction::Remove { .. } => chat
                    .members
                    .retain(|member| member != &record.target_profile),
            }
            if let Err(error) = self.save() {
                let previous_id = previous_chat.id;
                *self
                    .manifest
                    .chats
                    .iter_mut()
                    .find(|c| c.id == previous_id)
                    .unwrap() = previous_chat;
                return Err(error);
            }
            self.clients
                .get_mut(alias)
                .unwrap()
                .acknowledge_lifecycle(record.approval.context.request_id)?;
            record.completed = true;
            records[index] = record;
            self.save_lifecycle(&owner, &records)?;
        }
        Ok(())
    }
    pub async fn change_group_lifecycle(
        &mut self,
        alias: &str,
        conversation: ConversationId,
        request_id: MessageId,
        target_alias: &str,
        fingerprint: &str,
        remove: bool,
    ) -> Result<()> {
        let owner = self.profile(alias)?;
        let target = self.profile(target_alias)?;
        ensure!(
            fingerprint.len() <= 128
                && fingerprint
                    .split_whitespace()
                    .collect::<String>()
                    .to_lowercase()
                    == target.binding.fingerprint(),
            "Device fingerprint does not match."
        );
        let mut records = self.read_lifecycle(&owner)?;
        let action = if remove {
            LifecycleAction::Remove {
                target: target.principal.device_id,
            }
        } else {
            LifecycleAction::Transfer {
                target: target.principal.device_id,
            }
        };
        if let Some(previous) = records
            .iter()
            .find(|r| r.approval.context.request_id == request_id)
        {
            ensure!(
                previous.approval.context.conversation_id == conversation
                    && previous.approval.context.action == action
                    && previous.approval.target == target.binding,
                "Conflicting group lifecycle request."
            );
            return self.flush_group_lifecycles(alias).await;
        }
        let chat = self.member_chat(alias, conversation)?;
        ensure!(
            chat.kind == "group" && chat.owner == alias,
            "Only the conversation owner can change group access."
        );
        ensure!(
            self.online.get(alias) == Some(&true),
            "Reconnect before changing group access."
        );
        ensure!(
            target_alias != alias && chat.members.iter().any(|m| m == target_alias),
            "Select another current member."
        );
        ensure!(
            owner.principal.tenant_id == target.principal.tenant_id,
            "Device belongs to another business."
        );
        if !remove {
            ensure!(
                self.relay.authenticate(&target.token).await? == target.principal,
                "The new owner's device must be active."
            );
        }
        self.sync_profile(alias).await?;
        records = self.read_lifecycle(&owner)?;
        ensure!(
            records.len() < 10_000,
            "Group lifecycle journal has too many entries."
        );
        let chat = self.member_chat(alias, conversation)?;
        ensure!(chat.owner == alias, "Conversation ownership changed.");
        let expected_devices = chat
            .members
            .iter()
            .map(|m| self.profile(m).map(|p| p.principal.device_id))
            .collect::<Result<Vec<_>>>()?;
        records.push(Record {
            owner_profile: alias.into(),
            target_profile: target_alias.into(),
            approval: ApprovedLifecycle {
                context: LifecycleContext {
                    conversation_id: conversation,
                    request_id,
                    expected_owner: owner.principal.device_id,
                    expected_devices,
                    action,
                },
                authority: owner.binding.clone(),
                target: target.binding,
            },
            input: None,
            accepted: None,
            completed: false,
        });
        self.save_lifecycle(&owner, &records)?;
        self.flush_group_lifecycles(alias).await
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    request_id: MessageId,
    target_profile: String,
    fingerprint: String,
    action: String,
}
async fn change(
    State(workspace): State<Shared>,
    RoutePath((alias, conversation)): RoutePath<(String, ConversationId)>,
    Json(input): Json<Input>,
) -> std::result::Result<StatusCode, LocalError> {
    if input.action != "remove" && input.action != "transfer" {
        return Err(anyhow::anyhow!("Unknown group action.").into());
    }
    workspace
        .lock()
        .await
        .change_group_lifecycle(
            &alias,
            conversation,
            input.request_id,
            &input.target_profile,
            &input.fingerprint,
            input.action == "remove",
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) fn routes() -> Router<Shared> {
    Router::new().route(
        "/{profile}/conversations/{conversation}/lifecycle",
        post(change),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn transfer_recovery_and_removal_preserve_authority_and_exclude_old_device() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Workspace::open(dir.path().into()).await.unwrap();
        let group = w
            .manifest
            .chats
            .iter()
            .find(|c| c.title == "Launch team")
            .unwrap()
            .id;
        let bob = w.profile("bob").unwrap();
        let request = MessageId::new();
        w.change_group_lifecycle(
            "alice",
            group,
            request,
            "bob",
            &bob.binding.fingerprint(),
            false,
        )
        .await
        .unwrap();
        let alice = w.profile("alice").unwrap();
        let mut records = w.read_lifecycle(&alice).unwrap();
        let exact = serde_json::to_vec(&records[0].input).unwrap();
        records[0].accepted = None;
        records[0].completed = false;
        w.save_lifecycle(&alice, &records).unwrap();
        drop(w);
        let mut w = Workspace::open(dir.path().into()).await.unwrap();
        assert_eq!(w.member_chat("bob", group).unwrap().owner, "bob");
        assert_eq!(
            serde_json::to_vec(&w.read_lifecycle(&alice).unwrap()[0].input).unwrap(),
            exact
        );
        let maya = w.profile("maya").unwrap();
        assert!(
            w.change_group_lifecycle(
                "alice",
                group,
                MessageId::new(),
                "maya",
                &maya.binding.fingerprint(),
                true
            )
            .await
            .is_err()
        );
        w.change_group_lifecycle(
            "bob",
            group,
            MessageId::new(),
            "maya",
            &maya.binding.fingerprint(),
            true,
        )
        .await
        .unwrap();
        assert!(w.member_chat("maya", group).is_err());
        assert!(w.relay.sync(&maya.principal, group, 0, 32).await.is_err());
        w.send(
            "bob",
            group,
            MessageId::new(),
            Content {
                version: 1,
                text: "After member removal".into(),
                attachment: None,
            },
        )
        .await
        .unwrap();
        let snapshot = w.snapshot("alice").await.unwrap();
        assert!(
            snapshot["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == group.to_string())
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["text"] == "After member removal")
        );
        let recovery = croaker_client::backup::RecoveryCode::generate();
        let code = recovery.display();
        assert!(w.export_backup("alice", &code).is_ok());
    }
}
