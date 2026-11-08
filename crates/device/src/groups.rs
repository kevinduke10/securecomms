//! Manual trusted approval distribution for the independent device client.
use super::*;
use croaker_client::{ApprovedAddition, lifecycle::ApprovedLifecycle};
use croaker_protocol::{
    lifecycle::*,
    membership::{AcceptedAdmission, GroupAdmission},
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GroupDecision {
    version: u16,
    conversation_id: ConversationId,
    title: String,
    kind: String,
    authority: DeviceBinding,
    target: DeviceBinding,
    expected_devices: Vec<DeviceId>,
    request_id: MessageId,
    action: String,
    package: Option<String>,
    admission: Option<GroupAdmission>,
    lifecycle: Option<GroupLifecycle>,
    accepted_admission: Option<AcceptedAdmission>,
    accepted_lifecycle: Option<AcceptedLifecycle>,
    originated: bool,
    complete: bool,
}
impl DeviceRuntime {
    fn group_pin(&self, id: DeviceId) -> Result<DeviceBinding> {
        let p = self.profile()?;
        if id == p.binding.device_id {
            Ok(p.binding.clone())
        } else {
            Ok(p.contacts
                .get(&id)
                .context("Import and verify this contact first.")?
                .binding
                .clone())
        }
    }
    fn verify_group_decision(&self, record: &GroupDecision) -> Result<()> {
        ensure!(
            record.version == 1 && record.kind == "group" && record.title.len() <= 80,
            "Invalid group decision"
        );
        ensure!(
            self.group_pin(record.authority.device_id)? == record.authority
                && self.group_pin(record.target.device_id)? == record.target,
            "Group decision changed a pinned identity"
        );
        let mut expected = record.expected_devices.clone();
        expected.sort();
        expected.dedup();
        ensure!(
            expected.len() == record.expected_devices.len()
                && expected.contains(&record.authority.device_id),
            "Invalid prior roster"
        );
        for device in &expected {
            self.group_pin(*device)?;
        }
        match record.action.as_str() {
            "add" => {
                let admission = record
                    .admission
                    .as_ref()
                    .context("Missing saved admission")?;
                admission.validate().map_err(anyhow::Error::msg)?;
                ensure!(
                    admission.request_id == record.request_id
                        && admission.expected_devices == record.expected_devices
                        && admission.added_device == record.target.device_id
                        && record.accepted_admission.is_some(),
                    "Admission decision mismatch"
                );
            }
            "transfer" | "remove" => {
                let lifecycle = record
                    .lifecycle
                    .as_ref()
                    .context("Missing saved lifecycle")?;
                lifecycle.validate().map_err(anyhow::Error::msg)?;
                let context = &lifecycle.context;
                let expected_action = if record.action == "transfer" {
                    LifecycleAction::Transfer {
                        target: record.target.device_id,
                    }
                } else {
                    LifecycleAction::Remove {
                        target: record.target.device_id,
                    }
                };
                ensure!(
                    context.conversation_id == record.conversation_id
                        && context.request_id == record.request_id
                        && context.expected_owner == record.authority.device_id
                        && context.expected_devices == record.expected_devices
                        && context.action == expected_action
                        && record.accepted_lifecycle.is_some(),
                    "Lifecycle decision mismatch"
                );
            }
            _ => bail!("Unsupported group decision"),
        }
        Ok(())
    }
    pub(super) async fn groups_management(
        &mut self,
        method: &str,
        path: &str,
        body: &Value,
    ) -> Result<Option<Value>> {
        if method == "GET" && path == "/desktop/group-decisions" {
            return Ok(Some(json!(
                self.profile()?
                    .group_decisions
                    .iter()
                    .filter(|d| d.accepted_admission.is_some() || d.accepted_lifecycle.is_some())
                    .collect::<Vec<_>>()
            )));
        }
        if method != "POST" || path != "/desktop/group-decisions" {
            return Ok(None);
        }
        let mut record: GroupDecision = serde_json::from_value(body["decision"].clone())?;
        self.verify_group_decision(&record)?;
        let fingerprint = body["fingerprint"]
            .as_str()
            .context("Verify the approving owner's full fingerprint.")?
            .split_whitespace()
            .collect::<String>()
            .to_lowercase();
        ensure!(
            fingerprint == record.authority.fingerprint(),
            "Owner fingerprint mismatch"
        );
        if let Some(previous) = self
            .profile()?
            .group_decisions
            .iter()
            .find(|d| d.request_id == record.request_id)
        {
            ensure!(
                previous.conversation_id == record.conversation_id
                    && previous.admission == record.admission
                    && previous.lifecycle == record.lifecycle,
                "Conflicting group decision"
            );
            return Ok(Some(json!({})));
        }
        let own = self.profile()?.binding.device_id;
        if let Some(chat) = self.profile()?.chats.get(&record.conversation_id) {
            let mut current = chat.members.clone();
            current.sort();
            let mut expected = record.expected_devices.clone();
            expected.sort();
            ensure!(
                chat.owner == record.authority.device_id && current == expected,
                "Decision does not match the saved group owner and roster"
            );
        } else {
            ensure!(
                record.action == "add" && record.target.device_id == own,
                "Import the original conversation invitation first."
            );
            let mut members = record.expected_devices.clone();
            members.push(own);
            members.sort();
            self.private.as_mut().unwrap().chats.insert(
                record.conversation_id,
                Chat {
                    id: record.conversation_id,
                    title: record.title.clone(),
                    kind: record.kind.clone(),
                    owner: record.authority.device_id,
                    members,
                    created: true,
                },
            );
        }
        record.originated = false;
        record.complete = false;
        record.package = None;
        if let Some(admission) = &record.admission {
            self.private
                .as_mut()
                .unwrap()
                .approvals
                .push(ApprovedAddition {
                    commit_id: admission.commit.message_id,
                    authority: record.authority.clone(),
                    added: record.target.clone(),
                });
        }
        if let Some(lifecycle) = &record.lifecycle {
            self.private
                .as_mut()
                .unwrap()
                .lifecycles
                .push(ApprovedLifecycle {
                    context: lifecycle.context.clone(),
                    authority: record.authority.clone(),
                    target: record.target.clone(),
                });
        }
        self.private.as_mut().unwrap().group_decisions.push(record);
        self.save()?;
        self.prepare_group_joins()?;
        Ok(Some(json!({})))
    }
    fn prepare_group_joins(&mut self) -> Result<()> {
        let own = self.profile()?.binding.device_id;
        let pending = self
            .profile()?
            .group_decisions
            .iter()
            .filter(|r| {
                !r.originated && !r.complete && r.action == "add" && r.target.device_id == own
            })
            .cloned()
            .collect::<Vec<_>>();
        for record in pending {
            if !self
                .client()?
                .update(|d| Ok(d.has_conversation(record.conversation_id)))?
            {
                self.client()?.prepare_join(
                    record.conversation_id,
                    record
                        .accepted_admission
                        .context("Missing admission receipt")?
                        .commit_sequence,
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn reconcile_group_decisions(&mut self) -> Result<()> {
        let records = self.profile()?.group_decisions.clone();
        let own = self.profile()?.binding.device_id;
        for (index, record) in records.into_iter().enumerate() {
            if record.complete {
                continue;
            }
            let authenticated = if record.originated {
                record.accepted_admission.is_some() || record.accepted_lifecycle.is_some()
            } else if record.action == "add" && record.target.device_id == own {
                self.client()?
                    .update(|d| Ok(d.has_conversation(record.conversation_id)))?
            } else {
                self.client()?
                    .group_commit_applied(record.conversation_id, record.request_id)?
            };
            if !authenticated {
                continue;
            }
            let chat = self
                .private
                .as_mut()
                .unwrap()
                .chats
                .get_mut(&record.conversation_id)
                .context("Missing group")?;
            match record.action.as_str() {
                "add" => {
                    if !chat.members.contains(&record.target.device_id) {
                        chat.members.push(record.target.device_id);
                        chat.members.sort();
                    }
                }
                "transfer" => chat.owner = record.target.device_id,
                "remove" => chat.members.retain(|id| *id != record.target.device_id),
                _ => bail!("Invalid saved group action"),
            }
            self.private.as_mut().unwrap().group_decisions[index].complete = true;
            self.save()?;
        }
        Ok(())
    }
    /// Recover saved owner intents before any application ratchet can advance.
    /// No network access occurs; a saved client batch is reused after a crash
    /// between its SQLite transaction and the profile journal update.
    pub(super) fn prepare_group_intents(&mut self) -> Result<()> {
        let records = self.profile()?.group_decisions.clone();
        for (index, mut record) in records.into_iter().enumerate() {
            if !record.originated || record.complete {
                continue;
            }
            match record.action.as_str() {
                "add" if record.admission.is_none() => {
                    let package = BASE64_STANDARD.decode(
                        record
                            .package
                            .as_ref()
                            .context("Missing pinned key package")?,
                    )?;
                    record.admission = Some(self.client()?.queue_admission(
                        record.conversation_id,
                        record.request_id,
                        &record.expected_devices,
                        &package,
                        &record.target,
                    )?);
                }
                "transfer" | "remove" if record.lifecycle.is_none() => {
                    let action = if record.action == "transfer" {
                        LifecycleAction::Transfer {
                            target: record.target.device_id,
                        }
                    } else {
                        LifecycleAction::Remove {
                            target: record.target.device_id,
                        }
                    };
                    let context = LifecycleContext {
                        conversation_id: record.conversation_id,
                        request_id: record.request_id,
                        expected_owner: record.authority.device_id,
                        expected_devices: record.expected_devices.clone(),
                        action,
                    };
                    record.lifecycle =
                        Some(self.client()?.queue_lifecycle(&context, &record.target)?);
                }
                "add" | "transfer" | "remove" => continue,
                _ => bail!("Invalid saved group action"),
            }
            self.private.as_mut().unwrap().group_decisions[index] = record;
            self.save()?;
        }
        Ok(())
    }
    pub(super) async fn flush_group_decisions(&mut self) -> Result<()> {
        self.prepare_group_intents()?;
        self.prepare_group_joins()?;
        let records = self.profile()?.group_decisions.clone();
        for (index, mut record) in records.into_iter().enumerate() {
            if !record.originated || record.complete {
                continue;
            }
            if record.action == "add" {
                if record.accepted_admission.is_none() {
                    record.accepted_admission = Some(serde_json::from_value(
                        self.remote(
                            "POST",
                            &format!("/v1/conversations/{}/admissions", record.conversation_id),
                            Some(&serde_json::to_value(
                                record
                                    .admission
                                    .as_ref()
                                    .context("Missing saved admission")?,
                            )?),
                            false,
                        )
                        .await?,
                    )?);
                    self.private.as_mut().unwrap().group_decisions[index] = record.clone();
                    self.save()?;
                }
                self.client()?.acknowledge_admission(record.request_id)?;
            } else {
                if record.accepted_lifecycle.is_none() {
                    record.accepted_lifecycle = Some(serde_json::from_value(
                        self.remote(
                            "POST",
                            &format!("/v1/conversations/{}/lifecycle", record.conversation_id),
                            Some(&serde_json::to_value(
                                record
                                    .lifecycle
                                    .as_ref()
                                    .context("Missing saved lifecycle")?,
                            )?),
                            false,
                        )
                        .await?,
                    )?);
                    self.private.as_mut().unwrap().group_decisions[index] = record.clone();
                    self.save()?;
                }
                self.client()?.acknowledge_lifecycle(record.request_id)?;
            }
        }
        self.reconcile_group_decisions()?;
        Ok(())
    }
    pub(super) async fn group_request(
        &mut self,
        method: &str,
        route: &str,
        body: &Value,
    ) -> Result<Option<Value>> {
        let parts = route.split('/').collect::<Vec<_>>();
        if parts.len() != 3
            || parts[0] != "conversations"
            || !matches!(parts[2], "members" | "lifecycle")
        {
            return Ok(None);
        }
        let id: ConversationId = parts[1].parse()?;
        let chat = self
            .profile()?
            .chats
            .get(&id)
            .context("Unknown group")?
            .clone();
        let own = self.profile()?.binding.device_id;
        ensure!(
            chat.kind == "group",
            "Only groups support membership changes"
        );
        if method == "GET" && parts[2] == "members" {
            let p = self.profile()?;
            let all = std::iter::once((p.binding.device_id, p.name.clone(), p.binding.clone()))
                .chain(
                    p.contacts
                        .values()
                        .map(|c| (c.binding.device_id, c.name.clone(), c.binding.clone())),
                );
            let mut members = vec![];
            let mut candidates = vec![];
            for (id, name, binding) in all {
                let value = json!({"profile":id,"name":name,"label":if id==own{"This device"}else{"Verified device"},"fingerprint":binding.fingerprint(),"owner":id==chat.owner});
                if chat.members.contains(&id) {
                    members.push(value);
                } else {
                    candidates.push(value);
                }
            }
            return Ok(Some(
                json!({"can_invite":chat.owner==own&&self.online,"members":members,"candidates":candidates}),
            ));
        }
        ensure!(
            method == "POST" && self.online,
            "Reconnect as the conversation owner to change group access."
        );
        let target_id: DeviceId = serde_json::from_value(body["target_profile"].clone())?;
        let target = self.group_pin(target_id)?;
        let fingerprint = body["fingerprint"]
            .as_str()
            .context("Verify the target fingerprint.")?
            .split_whitespace()
            .collect::<String>()
            .to_lowercase();
        ensure!(
            fingerprint == target.fingerprint(),
            "Target fingerprint mismatch"
        );
        let request: MessageId = serde_json::from_value(body["request_id"].clone())?;
        let action = if parts[2] == "members" {
            "add"
        } else {
            body["action"].as_str().context("Choose a group action")?
        };
        ensure!(
            matches!(action, "add" | "transfer" | "remove") && target_id != own,
            "Invalid group action"
        );
        if let Some(previous) = self
            .profile()?
            .group_decisions
            .iter()
            .find(|r| r.request_id == request)
        {
            ensure!(
                previous.conversation_id == id
                    && previous.target == target
                    && previous.action == action,
                "Conflicting group request"
            );
            self.flush_group_decisions().await?;
            return Ok(Some(json!({})));
        }
        self.sync().await?;
        let chat = self
            .profile()?
            .chats
            .get(&id)
            .context("Unknown group")?
            .clone();
        ensure!(chat.owner == own, "Group owner changed");
        ensure!(
            chat.members.contains(&target_id) == (action != "add"),
            "Target does not match group membership"
        );
        ensure!(
            action != "add"
                || !self
                    .profile()?
                    .group_decisions
                    .iter()
                    .any(|r| r.conversation_id == id
                        && r.complete
                        && r.action == "remove"
                        && r.target.device_id == target_id),
            "Enroll a fresh device before rejoining a removed member."
        );
        let package = if action == "add" {
            Some(
                self.profile()?
                    .contacts
                    .get(&target_id)
                    .context("Import target contact")?
                    .package
                    .clone(),
            )
        } else {
            None
        };
        let authority = self.group_pin(own)?;
        self.private
            .as_mut()
            .unwrap()
            .group_decisions
            .push(GroupDecision {
                version: 1,
                conversation_id: id,
                title: chat.title,
                kind: chat.kind,
                authority,
                target,
                expected_devices: chat.members,
                request_id: request,
                action: action.into(),
                package,
                admission: None,
                lifecycle: None,
                accepted_admission: None,
                accepted_lifecycle: None,
                originated: true,
                complete: false,
            });
        self.save()?;
        self.flush_group_decisions().await?;
        Ok(Some(json!({})))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Secrets(Mutex<BTreeMap<String, Vec<u8>>>);
    impl SecretStore for Secrets {
        fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .map(Zeroizing::new))
        }
        fn set(&self, name: &str, value: &[u8]) -> Result<()> {
            self.0.lock().unwrap().insert(name.into(), value.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<()> {
            self.0.lock().unwrap().remove(name);
            Ok(())
        }
    }
    #[tokio::test]
    async fn imported_handoff_cannot_swap_owner_before_authenticated_commit() {
        let tenant = TenantId::new();
        let id = ConversationId::new();
        let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
        let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
        alice.create_conversation(id).unwrap();
        let add = alice
            .add_member(id, &bob.key_package().unwrap(), bob.binding())
            .unwrap();
        bob.join(id, &add.welcome, &[alice.binding().clone()])
            .unwrap();
        let authority = alice.binding().clone();
        let target = bob.binding().clone();
        let members = alice.conversation_devices(id).unwrap();
        let request_id = MessageId::new();
        let context = LifecycleContext {
            conversation_id: id,
            request_id,
            expected_owner: authority.device_id,
            expected_devices: members.clone(),
            action: LifecycleAction::Transfer {
                target: target.device_id,
            },
        };
        let bytes = alice.commit_lifecycle(&context, &target).unwrap();
        let decision = GroupDecision {
            version: 1,
            conversation_id: id,
            title: "Test group".into(),
            kind: "group".into(),
            authority: authority.clone(),
            target: target.clone(),
            expected_devices: members.clone(),
            request_id,
            action: "transfer".into(),
            package: None,
            admission: None,
            lifecycle: Some(GroupLifecycle {
                context,
                commit: SubmitEvent {
                    protocol_version: API_VERSION,
                    message_id: request_id,
                    kind: EventKind::Commit,
                    recipient: None,
                    ciphertext: BASE64_STANDARD.encode(bytes),
                },
            }),
            accepted_admission: None,
            accepted_lifecycle: Some(AcceptedLifecycle {
                sequence: 1,
                duplicate: false,
            }),
            originated: true,
            complete: true,
        };
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(Secrets::default());
        let mut runtime = DeviceRuntime::open(dir.path().into(), secrets.clone()).unwrap();
        runtime.client = Some(
            ClientStore::create(
                dir.path().join("client.sqlite"),
                bob,
                Zeroizing::new(*runtime.key),
            )
            .unwrap(),
        );
        let enrollment = EnrollmentInput {
            id: MessageId::new(),
            binding: IdentityBinding {
                tenant_id: tenant,
                device_id: target.device_id,
                signature_key: target.signature_key.clone(),
            },
            label: "Test".into(),
            token_hash: "00".repeat(32),
            proof: vec![],
        };
        runtime.connection = Some(Connection {
            relay: "https://native-fixture.example".into(),
        });
        atomic_write(
            &dir.path().join("connection.json"),
            &serde_json::to_vec(&runtime.connection).unwrap(),
        )
        .unwrap();
        runtime.private = Some(Private {
            relay: "https://native-fixture.example".into(),
            binding: target.clone(),
            token: "00".repeat(32),
            account_id: "a".repeat(64),
            name: "Bob".into(),
            role: Role::Member,
            enrollment,
            approved: true,
            locked: false,
            contacts: BTreeMap::from([(
                authority.device_id,
                Contact {
                    version: 1,
                    name: "Alice".into(),
                    binding: authority.clone(),
                    package: String::new(),
                },
            )]),
            chats: BTreeMap::from([(
                id,
                Chat {
                    id,
                    title: "Test group".into(),
                    kind: "group".into(),
                    owner: authority.device_id,
                    members,
                    created: true,
                },
            )]),
            read_receipts: false,
            approvals: vec![],
            lifecycles: vec![],
            group_decisions: vec![],
        });
        let mut import = json!({"decision":decision,"fingerprint":"incorrect"});
        assert!(
            runtime
                .groups_management("POST", "/desktop/group-decisions", &import)
                .await
                .is_err()
        );
        assert!(runtime.profile().unwrap().lifecycles.is_empty());
        import["fingerprint"] = json!(authority.fingerprint());
        runtime
            .groups_management("POST", "/desktop/group-decisions", &import)
            .await
            .unwrap();
        runtime.reconcile_group_decisions().unwrap();
        assert_eq!(
            runtime.profile().unwrap().chats[&id].owner,
            authority.device_id
        );
        // The authenticated decision survives restart, but remains non-authoritative
        // until this client's MLS transaction authenticates the exact commit.
        drop(runtime);
        let mut runtime = DeviceRuntime::open(dir.path().into(), secrets).unwrap();
        let approvals = runtime.profile().unwrap().lifecycles.clone();
        let event = decision.lifecycle.unwrap().commit;
        let page = SyncPage {
            events: vec![StoredEvent {
                sequence: 1,
                sender: authority.device_id,
                event,
                submitted_at: 1,
                expires_at: u64::MAX,
            }],
            next_cursor: 1,
            has_more: false,
        };
        runtime
            .client()
            .unwrap()
            .apply_page_with_lifecycle_report(id, 0, &page, &[], |_| false, &[], &approvals)
            .unwrap();
        runtime.reconcile_group_decisions().unwrap();
        assert_eq!(
            runtime.profile().unwrap().chats[&id].owner,
            target.device_id
        );
    }
    #[tokio::test]
    async fn restart_prepares_owner_intent_before_direct_offline_send() {
        let tenant = TenantId::new();
        let id = ConversationId::new();
        let mut alice = Device::new(tenant, DeviceId::new()).unwrap();
        let mut bob = Device::new(tenant, DeviceId::new()).unwrap();
        alice.create_conversation(id).unwrap();
        let addition = alice
            .add_member(id, &bob.key_package().unwrap(), bob.binding())
            .unwrap();
        bob.join(id, &addition.welcome, &[alice.binding().clone()])
            .unwrap();
        let authority = alice.binding().clone();
        let target = bob.binding().clone();
        let members = alice.conversation_devices(id).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(Secrets::default());
        let mut runtime = DeviceRuntime::open(dir.path().into(), secrets.clone()).unwrap();
        runtime.connection = Some(Connection {
            relay: "https://native-fixture.example".into(),
        });
        atomic_write(
            &runtime.directory.join("connection.json"),
            &serde_json::to_vec(&runtime.connection).unwrap(),
        )
        .unwrap();
        runtime.client = Some(
            ClientStore::create(
                dir.path().join("client.sqlite"),
                alice,
                Zeroizing::new(*runtime.key),
            )
            .unwrap(),
        );
        let request_id = MessageId::new();
        let intent = GroupDecision {
            version: 1,
            conversation_id: id,
            title: "Offline handoff".into(),
            kind: "group".into(),
            authority: authority.clone(),
            target: target.clone(),
            expected_devices: members.clone(),
            request_id,
            action: "transfer".into(),
            package: None,
            admission: None,
            lifecycle: None,
            accepted_admission: None,
            accepted_lifecycle: None,
            originated: true,
            complete: false,
        };
        let enrollment = EnrollmentInput {
            id: MessageId::new(),
            binding: IdentityBinding {
                tenant_id: tenant,
                device_id: authority.device_id,
                signature_key: authority.signature_key.clone(),
            },
            label: "Alice".into(),
            token_hash: "00".repeat(32),
            proof: vec![],
        };
        runtime.private = Some(Private {
            relay: "https://native-fixture.example".into(),
            binding: authority.clone(),
            token: "00".repeat(32),
            account_id: "a".repeat(64),
            name: "Alice".into(),
            role: Role::Member,
            enrollment,
            approved: true,
            locked: false,
            contacts: BTreeMap::from([(
                target.device_id,
                Contact {
                    version: 1,
                    name: "Bob".into(),
                    binding: target.clone(),
                    package: String::new(),
                },
            )]),
            chats: BTreeMap::from([(
                id,
                Chat {
                    id,
                    title: "Offline handoff".into(),
                    kind: "group".into(),
                    owner: authority.device_id,
                    members,
                    created: true,
                },
            )]),
            read_receipts: false,
            approvals: vec![],
            lifecycles: vec![],
            group_decisions: vec![intent],
        });
        runtime.save().unwrap();
        drop(runtime);
        // No snapshot or sync runs after restart. A direct offline send must
        // recover the saved handoff and advance the epoch before encrypting.
        let mut runtime = DeviceRuntime::open(dir.path().into(), secrets.clone()).unwrap();
        runtime.online = false;
        let message_id = MessageId::new();
        let response=runtime.request("POST",&format!("/local-api/{}/send",authority.device_id),Some(json!({"conversation_id":id,"request_id":message_id,"content":{"version":1,"text":"Prepared before this message","attachment":null}}))).await;
        assert_eq!(response.status, 200, "{}", response.body);
        let batch = runtime
            .client()
            .unwrap()
            .pending_lifecycles()
            .unwrap()
            .remove(0)
            .input;
        let message = runtime.client().unwrap().pending().unwrap().remove(0);
        assert_eq!(message.event.message_id, message_id);
        bob.apply_lifecycle(
            &batch.context,
            &BASE64_STANDARD.decode(&batch.commit.ciphertext).unwrap(),
            &authority,
            &target,
        )
        .unwrap();
        let croaker_crypto::Received::Application { plaintext, .. } = bob
            .receive(
                id,
                &BASE64_STANDARD.decode(&message.event.ciphertext).unwrap(),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&plaintext).unwrap()["text"],
            "Prepared before this message"
        );
        // Model a lost profile-journal write after SQLite saved its batch.
        // Retry must find that exact batch even with a newer app in the outbox.
        runtime.private.as_mut().unwrap().group_decisions[0].lifecycle = None;
        runtime.save().unwrap();
        drop(runtime);
        let mut runtime = DeviceRuntime::open(dir.path().into(), secrets).unwrap();
        runtime.online = false;
        runtime.prepare_group_intents().unwrap();
        assert_eq!(
            runtime
                .client()
                .unwrap()
                .pending_lifecycles()
                .unwrap()
                .remove(0)
                .input,
            batch
        );
        assert_eq!(
            runtime.client().unwrap().pending().unwrap().remove(0),
            message
        );
    }
}
