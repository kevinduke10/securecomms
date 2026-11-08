//! DynamoDB adapter. Conditional transactions serialize concurrent appends and
//! atomically persist the message and its deduplication receipt.
use crate::*;
use async_trait::async_trait;
use aws_sdk_dynamodb::{
    Client,
    types::{
        AttributeValue as A, ConditionCheck, KeysAndAttributes, Put, TransactWriteItem, Update,
    },
};
use croaker_protocol::*;
use std::collections::HashMap;

type Item = HashMap<String, A>;

#[derive(Clone)]
pub struct DynamoStore {
    client: Client,
    table: String,
}

fn key(pk: &str, sk: &str) -> Item {
    HashMap::from([
        ("pk".into(), A::S(pk.into())),
        ("sk".into(), A::S(sk.into())),
    ])
}
fn tenant_key(tenant: TenantId) -> String {
    format!("TENANT#{tenant}")
}
fn group_key(tenant: TenantId, id: ConversationId) -> String {
    format!("TENANT#{tenant}#CONVERSATION#{id}")
}
fn device_key(device: DeviceId) -> String {
    format!("DEVICE#{device}")
}
fn string<'a>(item: &'a Item, field: &str) -> Result<&'a str> {
    item.get(field)
        .and_then(|a| a.as_s().ok())
        .map(String::as_str)
        .ok_or(StoreError::Unavailable)
}
fn number(item: &Item, field: &str) -> Result<u64> {
    item.get(field)
        .and_then(|a| a.as_n().ok())
        .and_then(|s| s.parse().ok())
        .ok_or(StoreError::Unavailable)
}
fn json<T: serde::de::DeserializeOwned>(item: &Item, field: &str) -> Result<T> {
    serde_json::from_str(string(item, field)?).map_err(|_| StoreError::Unavailable)
}
fn encoded<T: serde::Serialize>(value: &T) -> Result<A> {
    Ok(A::S(
        serde_json::to_string(value).map_err(|_| StoreError::Unavailable)?,
    ))
}

impl DynamoStore {
    pub fn new(client: Client, table: String) -> Self {
        Self { client, table }
    }

    async fn get(&self, pk: &str, sk: &str) -> Result<Option<Item>> {
        self.client
            .get_item()
            .table_name(&self.table)
            .set_key(Some(key(pk, sk)))
            .consistent_read(true)
            .send()
            .await
            .map(|r| r.item)
            .map_err(|_| StoreError::Unavailable)
    }

    fn account_update(
        &self,
        id: &str,
        previous: A,
        record: &identity::AccountRecord,
    ) -> Result<TransactWriteItem> {
        let update = Update::builder()
            .table_name(&self.table)
            .set_key(Some(key(&format!("ACCOUNT#{id}"), "IDENTITY")))
            .update_expression("SET #doc = :next")
            .condition_expression("#doc = :previous")
            .expression_attribute_names("#doc", "document")
            .expression_attribute_values(":next", encoded(record)?)
            .expression_attribute_values(":previous", previous)
            .build()
            .map_err(|_| StoreError::Unavailable)?;
        Ok(TransactWriteItem::builder().update(update).build())
    }
    async fn active(&self, actor: &Principal) -> Result<()> {
        let item = self
            .get(&tenant_key(actor.tenant_id), &device_key(actor.device_id))
            .await?
            .ok_or(StoreError::Unauthorized)?;
        let stored: Principal = json(&item, "principal")?;
        if stored != *actor || item.get("revoked") != Some(&A::Bool(false)) {
            return Err(StoreError::Unauthorized);
        }
        Ok(())
    }

    fn guard(&self, actor: &Principal) -> Result<TransactWriteItem> {
        let check = ConditionCheck::builder()
            .table_name(&self.table)
            .set_key(Some(key(
                &tenant_key(actor.tenant_id),
                &device_key(actor.device_id),
            )))
            .condition_expression("principal = :principal AND revoked = :no")
            .expression_attribute_values(":principal", encoded(actor)?)
            .expression_attribute_values(":no", A::Bool(false))
            .build()
            .map_err(|_| StoreError::Unavailable)?;
        Ok(TransactWriteItem::builder().condition_check(check).build())
    }

    async fn conversation(
        &self,
        actor: &Principal,
        id: ConversationId,
    ) -> Result<(Conversation, u64)> {
        self.active(actor).await?;
        let item = self
            .get(&group_key(actor.tenant_id, id), "META")
            .await?
            .ok_or(StoreError::NotFound)?;
        let conversation: Conversation = json(&item, "metadata")?;
        if !conversation.devices.contains(&actor.device_id) {
            return Err(StoreError::NotFound);
        }
        Ok((conversation, number(&item, "sequence")?))
    }

    async fn verify_devices(&self, tenant: TenantId, devices: &[DeviceId]) -> Result<()> {
        for chunk in devices.chunks(100) {
            let request = KeysAndAttributes::builder()
                .set_keys(Some(
                    chunk
                        .iter()
                        .map(|id| key(&tenant_key(tenant), &device_key(*id)))
                        .collect(),
                ))
                .consistent_read(true)
                .build()
                .map_err(|_| StoreError::Unavailable)?;
            let result = self
                .client
                .batch_get_item()
                .request_items(&self.table, request)
                .send()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            if result.unprocessed_keys().is_some_and(|m| !m.is_empty()) {
                return Err(StoreError::Unavailable);
            }
            let rows = result
                .responses()
                .and_then(|m| m.get(&self.table))
                .ok_or(StoreError::NotFound)?;
            if rows.len() != chunk.len()
                || rows
                    .iter()
                    .any(|row| row.get("revoked") != Some(&A::Bool(false)))
            {
                return Err(StoreError::NotFound);
            }
        }
        Ok(())
    }

    fn put(&self, item: Item) -> Result<TransactWriteItem> {
        Ok(TransactWriteItem::builder()
            .put(
                Put::builder()
                    .table_name(&self.table)
                    .set_item(Some(item))
                    .condition_expression("attribute_not_exists(pk)")
                    .build()
                    .map_err(|_| StoreError::Unavailable)?,
            )
            .build())
    }
}

#[async_trait]
impl Store for DynamoStore {
    async fn revoke_owned_device(
        &self,
        actor: &Principal,
        account_id: &str,
        device: DeviceId,
    ) -> Result<()> {
        self.active(actor).await?;
        let mut record = self.account(account_id).await?;
        let previous = encoded(&record)?;
        record.can_approve(actor)?;
        if !record.devices.iter().any(|d| d.binding.device_id == device) {
            return Err(StoreError::NotFound);
        }
        for entry in &mut record.devices {
            if entry.binding.device_id == device {
                entry.revoked = true;
            }
        }
        let mut update = Update::builder()
            .table_name(&self.table)
            .set_key(Some(key(&tenant_key(actor.tenant_id), &device_key(device))))
            .update_expression("SET revoked = :yes")
            .expression_attribute_values(":yes", A::Bool(true));
        let mut writes = vec![self.account_update(account_id, previous, &record)?];
        if actor.device_id == device {
            update = update
                .condition_expression("principal = :principal AND revoked = :no")
                .expression_attribute_values(":principal", encoded(actor)?)
                .expression_attribute_values(":no", A::Bool(false));
        } else {
            writes.push(self.guard(actor)?);
            update = update.condition_expression("attribute_exists(pk)");
        }
        writes.push(
            TransactWriteItem::builder()
                .update(update.build().map_err(|_| StoreError::Unavailable)?)
                .build(),
        );
        self.client
            .transact_write_items()
            .set_transact_items(Some(writes))
            .send()
            .await
            .map_err(|_| StoreError::Conflict)?;
        Ok(())
    }
    async fn grant_account(&self, grant: identity::AccountGrant) -> Result<()> {
        let record = identity::AccountRecord::new(grant)?;
        let pk = format!("ACCOUNT#{}", record.grant.id());
        if let Some(existing) = self.get(&pk, "IDENTITY").await? {
            let previous: identity::AccountRecord = json(&existing, "document")?;
            if previous.grant == record.grant {
                return Ok(());
            }
            return Err(StoreError::Conflict);
        }
        let mut item = key(&pk, "IDENTITY");
        item.insert("document".into(), encoded(&record)?);
        let mut writes = vec![self.put(item)?];
        if let Some(binding) = &record.grant.bootstrap_device {
            writes.push(self.guard(&Principal {
                tenant_id: record.grant.tenant_id,
                device_id: binding.device_id,
                role: record.grant.role,
            })?);
        }
        self.client
            .transact_write_items()
            .set_transact_items(Some(writes))
            .send()
            .await
            .map_err(|_| StoreError::Conflict)?;
        Ok(())
    }
    async fn account(&self, id: &str) -> Result<identity::AccountRecord> {
        let item = self
            .get(&format!("ACCOUNT#{id}"), "IDENTITY")
            .await?
            .ok_or(StoreError::Unauthorized)?;
        let record: identity::AccountRecord = json(&item, "document")?;
        record.active()?;
        Ok(record)
    }
    async fn request_enrollment(
        &self,
        id: &str,
        input: croaker_protocol::enrollment::EnrollmentInput,
    ) -> Result<()> {
        let mut record = self.account(id).await?;
        let previous = encoded(&record)?;
        record.request(input)?;
        self.client
            .transact_write_items()
            .transact_items(self.account_update(id, previous, &record)?)
            .send()
            .await
            .map_err(|_| StoreError::Conflict)?;
        Ok(())
    }
    async fn decide_enrollment(
        &self,
        actor: &Principal,
        account_id: &str,
        id: MessageId,
        fingerprint: &str,
        approve: bool,
    ) -> Result<()> {
        self.active(actor).await?;
        let mut record = self.account(account_id).await?;
        let previous = encoded(&record)?;
        let credential = record.decide(actor, id, fingerprint, approve)?;
        let mut writes = vec![
            self.guard(actor)?,
            self.account_update(account_id, previous, &record)?,
        ];
        if let Some((principal, hash)) = credential {
            let mut device = key(
                &tenant_key(principal.tenant_id),
                &device_key(principal.device_id),
            );
            device.insert("principal".into(), encoded(&principal)?);
            device.insert("revoked".into(), A::Bool(false));
            let mut auth = key(&format!("TOKEN#{hash}"), "AUTH");
            auth.insert("principal".into(), encoded(&principal)?);
            writes.extend([self.put(device)?, self.put(auth)?]);
        }
        self.client
            .transact_write_items()
            .set_transact_items(Some(writes))
            .send()
            .await
            .map_err(|_| StoreError::Conflict)?;
        Ok(())
    }
    async fn authenticate(&self, token: &str) -> Result<Principal> {
        let auth = self
            .get(&format!("TOKEN#{}", token_hash(token)), "AUTH")
            .await?
            .ok_or(StoreError::Unauthorized)?;
        let principal: Principal = json(&auth, "principal")?;
        self.active(&principal).await?;
        Ok(principal)
    }

    async fn provision(&self, actor: &Principal, token: &str) -> Result<()> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StoreError::Invalid(
                "use a random 32-byte hexadecimal token",
            ));
        }
        let mut device = key(&tenant_key(actor.tenant_id), &device_key(actor.device_id));
        device.insert("principal".into(), encoded(actor)?);
        device.insert("revoked".into(), A::Bool(false));
        let mut auth = key(&format!("TOKEN#{}", token_hash(token)), "AUTH");
        auth.insert("principal".into(), encoded(actor)?);
        self.client
            .transact_write_items()
            .transact_items(self.put(device)?)
            .transact_items(self.put(auth)?)
            .send()
            .await
            .map_err(|e| {
                if e.as_service_error()
                    .is_some_and(|e| e.is_transaction_canceled_exception())
                {
                    StoreError::Conflict
                } else {
                    StoreError::Unavailable
                }
            })?;
        Ok(())
    }

    async fn revoke(&self, actor: &Principal, device: DeviceId) -> Result<()> {
        self.active(actor).await?;
        if actor.role != Role::Admin && actor.device_id != device {
            return Err(StoreError::NotFound);
        }
        if self
            .get(&tenant_key(actor.tenant_id), &device_key(device))
            .await?
            .is_none()
        {
            return Err(StoreError::NotFound);
        }
        let mut update = Update::builder()
            .table_name(&self.table)
            .set_key(Some(key(&tenant_key(actor.tenant_id), &device_key(device))))
            .update_expression("SET revoked = :yes")
            .expression_attribute_values(":yes", A::Bool(true));
        let mut tx = self.client.transact_write_items();
        if actor.device_id == device {
            update = update
                .condition_expression("principal = :principal AND revoked = :no")
                .expression_attribute_values(":principal", encoded(actor)?)
                .expression_attribute_values(":no", A::Bool(false));
        } else {
            tx = tx.transact_items(self.guard(actor)?);
            update = update.condition_expression("attribute_exists(pk)");
        }
        tx.transact_items(
            TransactWriteItem::builder()
                .update(update.build().map_err(|_| StoreError::Unavailable)?)
                .build(),
        )
        .send()
        .await
        .map_err(|_| StoreError::Unavailable)?;
        Ok(())
    }

    async fn create(&self, actor: &Principal, input: CreateConversation) -> Result<Conversation> {
        let devices = validate_conversation(actor, &input)?;
        self.active(actor).await?;
        self.verify_devices(actor.tenant_id, &devices).await?;
        let conversation = Conversation {
            id: input.id,
            tenant_id: actor.tenant_id,
            owner: actor.device_id,
            devices,
        };
        let pk = group_key(actor.tenant_id, input.id);
        let mut item = key(&pk, "META");
        item.insert("metadata".into(), encoded(&conversation)?);
        item.insert("sequence".into(), A::N("0".into()));
        let result = self
            .client
            .transact_write_items()
            .transact_items(self.guard(actor)?)
            .transact_items(self.put(item)?)
            .send()
            .await;
        if result.is_err() {
            self.active(actor).await?;
            if let Some(existing) = self.get(&pk, "META").await? {
                let existing: Conversation = json(&existing, "metadata")?;
                return if existing == conversation {
                    Ok(existing)
                } else {
                    Err(StoreError::Conflict)
                };
            }
            return Err(StoreError::Unavailable);
        }
        Ok(conversation)
    }

    async fn conversation_info(
        &self,
        actor: &Principal,
        id: ConversationId,
    ) -> Result<Conversation> {
        self.conversation(actor, id).await.map(|(group, _)| group)
    }
    async fn admit(
        &self,
        actor: &Principal,
        id: ConversationId,
        input: membership::GroupAdmission,
    ) -> Result<membership::AcceptedAdmission> {
        input.validate().map_err(StoreError::Invalid)?;
        let pk = group_key(actor.tenant_id, id);
        let receipt_key = format!("ADMISSION#{}", input.request_id);
        let fingerprint = admission_hash(&input);
        for _ in 0..8 {
            let (group, current) = self.conversation(actor, id).await?;
            if group.owner != actor.device_id {
                return Err(StoreError::NotFound);
            }
            if let Some(receipt) = self.get(&pk, &receipt_key).await? {
                self.active(actor).await?;
                return if string(&receipt, "fingerprint")? == fingerprint {
                    Ok(membership::AcceptedAdmission {
                        commit_sequence: number(&receipt, "commit_sequence")?,
                        welcome_sequence: number(&receipt, "welcome_sequence")?,
                        duplicate: true,
                    })
                } else {
                    Err(StoreError::Conflict)
                };
            }
            let next = validate_admission(actor, &group, &input)?;
            self.verify_devices(actor.tenant_id, &[input.added_device])
                .await?;
            let update = Update::builder()
                .table_name(&self.table)
                .set_key(Some(key(&pk, "META")))
                .update_expression("SET #seq = :next, metadata = :metadata")
                .condition_expression("#seq = :previous AND metadata = :old_metadata")
                .expression_attribute_names("#seq", "sequence")
                .expression_attribute_values(":next", A::N((current + 2).to_string()))
                .expression_attribute_values(":previous", A::N(current.to_string()))
                .expression_attribute_values(":metadata", encoded(&next)?)
                .expression_attribute_values(":old_metadata", encoded(&group)?)
                .build()
                .map_err(|_| StoreError::Unavailable)?;
            let added_guard = ConditionCheck::builder()
                .table_name(&self.table)
                .set_key(Some(key(
                    &tenant_key(actor.tenant_id),
                    &device_key(input.added_device),
                )))
                .condition_expression("revoked = :no")
                .expression_attribute_values(":no", A::Bool(false))
                .build()
                .map_err(|_| StoreError::Unavailable)?;
            let mut writes = vec![
                self.guard(actor)?,
                TransactWriteItem::builder()
                    .condition_check(added_guard)
                    .build(),
                TransactWriteItem::builder().update(update).build(),
            ];
            let timestamp = now();
            for (offset, event) in [(1, input.commit.clone()), (2, input.welcome.clone())] {
                let stored = StoredEvent {
                    sequence: current + offset,
                    sender: actor.device_id,
                    submitted_at: timestamp,
                    expires_at: timestamp + RETENTION_SECONDS,
                    event,
                };
                let mut item = key(&pk, &format!("EVENT#{:020}", stored.sequence));
                item.insert("payload".into(), encoded(&stored)?);
                item.insert("expires_at".into(), A::N(stored.expires_at.to_string()));
                writes.push(self.put(item)?);
                let mut receipt = key(
                    &pk,
                    &format!("RECEIPT#{}#{}", actor.device_id, stored.event.message_id),
                );
                receipt.insert("sequence".into(), A::N(stored.sequence.to_string()));
                receipt.insert("fingerprint".into(), A::S(event_hash(&stored.event)));
                writes.push(self.put(receipt)?);
            }
            let mut receipt = key(&pk, &receipt_key);
            receipt.insert("fingerprint".into(), A::S(fingerprint.clone()));
            receipt.insert("commit_sequence".into(), A::N((current + 1).to_string()));
            receipt.insert("welcome_sequence".into(), A::N((current + 2).to_string()));
            writes.push(self.put(receipt)?);
            let mut floor = key(&pk, &format!("JOIN#{}", input.added_device));
            floor.insert("after_sequence".into(), A::N((current + 1).to_string()));
            writes.push(self.put(floor)?);
            match self
                .client
                .transact_write_items()
                .set_transact_items(Some(writes))
                .send()
                .await
            {
                Ok(_) => {
                    return Ok(membership::AcceptedAdmission {
                        commit_sequence: current + 1,
                        welcome_sequence: current + 2,
                        duplicate: false,
                    });
                }
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|error| error.is_transaction_canceled_exception()) =>
                {
                    continue;
                }
                Err(_) => return Err(StoreError::Unavailable),
            }
        }
        Err(StoreError::Conflict)
    }
    async fn lifecycle(
        &self,
        actor: &Principal,
        id: ConversationId,
        input: croaker_protocol::lifecycle::GroupLifecycle,
    ) -> Result<croaker_protocol::lifecycle::AcceptedLifecycle> {
        use croaker_protocol::lifecycle::{AcceptedLifecycle, LifecycleAction};
        input.validate().map_err(StoreError::Invalid)?;
        let pk = group_key(actor.tenant_id, id);
        let receipt_key = format!("LIFECYCLE#{}", input.context.request_id);
        let fingerprint = crate::lifecycle::fingerprint(&input);
        for _ in 0..8 {
            self.active(actor).await?;
            if let Some(receipt) = self.get(&pk, &receipt_key).await? {
                return if string(&receipt, "actor")? == actor.device_id.to_string()
                    && string(&receipt, "fingerprint")? == fingerprint
                {
                    Ok(AcceptedLifecycle {
                        sequence: number(&receipt, "sequence")?,
                        duplicate: true,
                    })
                } else {
                    Err(StoreError::Conflict)
                };
            }
            let (group, current) = self.conversation(actor, id).await?;
            let next = crate::lifecycle::validate(actor, &group, id, &input)?;
            let update = Update::builder()
                .table_name(&self.table)
                .set_key(Some(key(&pk, "META")))
                .update_expression("SET #seq = :next, metadata = :metadata")
                .condition_expression("#seq = :previous AND metadata = :old_metadata")
                .expression_attribute_names("#seq", "sequence")
                .expression_attribute_values(":next", A::N((current + 1).to_string()))
                .expression_attribute_values(":previous", A::N(current.to_string()))
                .expression_attribute_values(":metadata", encoded(&next)?)
                .expression_attribute_values(":old_metadata", encoded(&group)?)
                .build()
                .map_err(|_| StoreError::Unavailable)?;
            let mut writes = vec![
                self.guard(actor)?,
                TransactWriteItem::builder().update(update).build(),
            ];
            if let LifecycleAction::Transfer { target } = input.context.action {
                self.verify_devices(actor.tenant_id, &[target]).await?;
                let guard = ConditionCheck::builder()
                    .table_name(&self.table)
                    .set_key(Some(key(&tenant_key(actor.tenant_id), &device_key(target))))
                    .condition_expression("revoked = :no")
                    .expression_attribute_values(":no", A::Bool(false))
                    .build()
                    .map_err(|_| StoreError::Unavailable)?;
                writes.push(TransactWriteItem::builder().condition_check(guard).build());
            }
            let timestamp = now();
            let stored = StoredEvent {
                sequence: current + 1,
                sender: actor.device_id,
                submitted_at: timestamp,
                expires_at: timestamp + RETENTION_SECONDS,
                event: input.commit.clone(),
            };
            let mut item = key(&pk, &format!("EVENT#{:020}", stored.sequence));
            item.insert("payload".into(), encoded(&stored)?);
            item.insert("expires_at".into(), A::N(stored.expires_at.to_string()));
            writes.push(self.put(item)?);
            let mut receipt = key(&pk, &receipt_key);
            receipt.insert("fingerprint".into(), A::S(fingerprint.clone()));
            receipt.insert("actor".into(), A::S(actor.device_id.to_string()));
            receipt.insert("sequence".into(), A::N(stored.sequence.to_string()));
            writes.push(self.put(receipt)?);
            let mut dedup = key(
                &pk,
                &format!("RECEIPT#{}#{}", actor.device_id, stored.event.message_id),
            );
            dedup.insert("sequence".into(), A::N(stored.sequence.to_string()));
            dedup.insert("fingerprint".into(), A::S(event_hash(&stored.event)));
            writes.push(self.put(dedup)?);
            match self
                .client
                .transact_write_items()
                .set_transact_items(Some(writes))
                .send()
                .await
            {
                Ok(_) => {
                    return Ok(AcceptedLifecycle {
                        sequence: stored.sequence,
                        duplicate: false,
                    });
                }
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|e| e.is_transaction_canceled_exception()) =>
                {
                    continue;
                }
                Err(_) => return Err(StoreError::Unavailable),
            }
        }
        Err(StoreError::Conflict)
    }
    async fn append(
        &self,
        actor: &Principal,
        id: ConversationId,
        input: SubmitEvent,
    ) -> Result<AcceptedEvent> {
        input.validate().map_err(StoreError::Invalid)?;
        let pk = group_key(actor.tenant_id, id);
        let receipt_key = format!("RECEIPT#{}#{}", actor.device_id, input.message_id);
        let fingerprint = event_hash(&input);
        for _ in 0..8 {
            let (conversation, current) = self.conversation(actor, id).await?;
            validate_event(actor, &conversation, &input)?;
            if let Some(previous) = self.get(&pk, &receipt_key).await? {
                self.active(actor).await?;
                return if string(&previous, "fingerprint")? == fingerprint {
                    Ok(AcceptedEvent {
                        sequence: number(&previous, "sequence")?,
                        duplicate: true,
                    })
                } else {
                    Err(StoreError::Conflict)
                };
            }
            let sequence = current + 1;
            let timestamp = now();
            let event = StoredEvent {
                sequence,
                sender: actor.device_id,
                submitted_at: timestamp,
                expires_at: timestamp + RETENTION_SECONDS,
                event: input.clone(),
            };
            let mut item = key(&pk, &format!("EVENT#{sequence:020}"));
            item.insert("payload".into(), encoded(&event)?);
            item.insert("expires_at".into(), A::N(event.expires_at.to_string()));
            let mut receipt = key(&pk, &receipt_key);
            receipt.insert("sequence".into(), A::N(sequence.to_string()));
            receipt.insert("fingerprint".into(), A::S(fingerprint.clone()));
            let update = Update::builder()
                .table_name(&self.table)
                .set_key(Some(key(&pk, "META")))
                .update_expression("SET #seq = :next")
                .condition_expression("#seq = :previous")
                .expression_attribute_names("#seq", "sequence")
                .expression_attribute_values(":next", A::N(sequence.to_string()))
                .expression_attribute_values(":previous", A::N(current.to_string()))
                .build()
                .map_err(|_| StoreError::Unavailable)?;
            let result = self
                .client
                .transact_write_items()
                .transact_items(self.guard(actor)?)
                .transact_items(TransactWriteItem::builder().update(update).build())
                .transact_items(self.put(item)?)
                .transact_items(self.put(receipt)?)
                .send()
                .await;
            match result {
                Ok(_) => {
                    return Ok(AcceptedEvent {
                        sequence,
                        duplicate: false,
                    });
                }
                Err(e)
                    if e.as_service_error()
                        .is_some_and(|e| e.is_transaction_canceled_exception()) =>
                {
                    continue;
                }
                Err(_) => return Err(StoreError::Unavailable),
            }
        }
        Err(StoreError::Conflict)
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
        let (_, latest) = self.conversation(actor, id).await?;
        let floor = self
            .get(
                &group_key(actor.tenant_id, id),
                &format!("JOIN#{}", actor.device_id),
            )
            .await?
            .map(|item| number(&item, "after_sequence"))
            .transpose()?
            .unwrap_or(0);
        let cursor = cursor.max(floor);
        if cursor > latest {
            return Err(StoreError::Invalid("cursor is ahead of conversation"));
        }
        let result = self
            .client
            .query()
            .table_name(&self.table)
            .key_condition_expression("pk = :pk AND sk BETWEEN :start AND :end")
            .expression_attribute_values(":pk", A::S(group_key(actor.tenant_id, id)))
            .expression_attribute_values(":start", A::S(format!("EVENT#{:020}", cursor + 1)))
            .expression_attribute_values(":end", A::S("EVENT#99999999999999999999".into()))
            .consistent_read(true)
            .limit((limit + 1) as i32)
            .send()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let events: Vec<StoredEvent> = result
            .items()
            .iter()
            .map(|row| json(row, "payload"))
            .collect::<Result<_>>()?;
        let mut page = page(events, cursor, limit, actor.device_id);
        page.has_more |= result.last_evaluated_key().is_some_and(|m| !m.is_empty());
        if !page.has_more {
            page.next_cursor = page.next_cursor.max(latest);
        }
        self.active(actor).await?;
        Ok(page)
    }
}
