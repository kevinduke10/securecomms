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
