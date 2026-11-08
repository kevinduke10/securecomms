//! Atomic object reservations and daily byte quotas in the existing relay table.
use super::*;
use aws_sdk_dynamodb::{
    Client,
    types::{AttributeValue as A, Put, TransactWriteItem, Update},
};
pub struct DynamoRegistry {
    client: Client,
    table: String,
}
impl DynamoRegistry {
    pub fn new(client: Client, table: String) -> Self {
        Self { client, table }
    }
}
#[async_trait]
impl Registry for DynamoRegistry {
    async fn find(&self, t: TenantId, c: ConversationId, o: MessageId) -> Result<Reservation> {
        let item = self
            .client
            .get_item()
            .table_name(&self.table)
            .key("pk", A::S(format!("MEDIA#{t}/{c}/{o}")))
            .key("sk", A::S("OBJECT".into()))
            .consistent_read(true)
            .send()
            .await
            .map_err(|_| StoreError::Unavailable)?
            .item
            .ok_or(StoreError::NotFound)?;
        let r: Reservation = serde_json::from_str(
            item.get("record")
                .and_then(|a| a.as_s().ok())
                .ok_or(StoreError::Unavailable)?,
        )
        .map_err(|_| StoreError::Unavailable)?;
        if r.expires_at <= now() {
            return Err(StoreError::NotFound);
        }
        Ok(r)
    }
    async fn reserve(
        &self,
        a: &Principal,
        c: ConversationId,
        u: ObjectUpload,
    ) -> Result<Reservation> {
        u.validate().map_err(StoreError::Invalid)?;
        match self.find(a.tenant_id, c, u.object_id).await {
            Ok(old) => {
                return if old.uploader == a.device_id && old.upload == u {
                    Ok(old)
                } else {
                    Err(StoreError::Conflict)
                };
            }
            Err(StoreError::NotFound) => {}
            Err(e) => return Err(e),
        }
        let r = Reservation {
            tenant: a.tenant_id,
            conversation: c,
            uploader: a.device_id,
            upload: u,
            expires_at: now() + RETENTION_SECONDS,
        };
        let put = Put::builder()
            .table_name(&self.table)
            .item("pk", A::S(format!("MEDIA#{}", r.key())))
            .item("sk", A::S("OBJECT".into()))
            .item(
                "record",
                A::S(serde_json::to_string(&r).map_err(|_| StoreError::Unavailable)?),
            )
            .item("expires_at", A::N(r.expires_at.to_string()))
            .condition_expression("attribute_not_exists(pk)")
            .build()
            .map_err(|_| StoreError::Unavailable)?;
        let mut items = vec![TransactWriteItem::builder().put(put).build()];
        for (key, limit) in [
            (
                format!("{}:{}:{}", a.tenant_id, a.device_id, now() / 86400),
                250u64 * 1024 * 1024,
            ),
            (
                format!("{}:{}", a.tenant_id, now() / 86400),
                1024u64 * 1024 * 1024,
            ),
        ] {
            let max = limit - r.upload.ciphertext_size;
            let update = Update::builder()
                .table_name(&self.table)
                .key("pk", A::S(format!("MEDIAQUOTA#{key}")))
                .key("sk", A::S("QUOTA".into()))
                .update_expression(
                    "SET #b = if_not_exists(#b, :zero) + :size, expires_at = :expiry",
                )
                .condition_expression("attribute_not_exists(#b) OR #b <= :max")
                .expression_attribute_names("#b", "bytes")
                .expression_attribute_values(":zero", A::N("0".into()))
                .expression_attribute_values(":size", A::N(r.upload.ciphertext_size.to_string()))
                .expression_attribute_values(":max", A::N(max.to_string()))
                .expression_attribute_values(":expiry", A::N((now() + 2 * 86400).to_string()))
                .build()
                .map_err(|_| StoreError::Unavailable)?;
            items.push(TransactWriteItem::builder().update(update).build());
        }
        match self
            .client
            .transact_write_items()
            .set_transact_items(Some(items))
            .send()
            .await
        {
            Ok(_) => Ok(r),
            Err(error) => {
                match self.find(a.tenant_id, c, r.upload.object_id).await {
                    Ok(old) if old.uploader == a.device_id && old.upload == r.upload => {
                        return Ok(old);
                    }
                    Ok(_) => return Err(StoreError::Conflict),
                    Err(StoreError::NotFound) => {}
                    Err(e) => return Err(e),
                }
                use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
                Err(match error.as_service_error() {
                    Some(TransactWriteItemsError::TransactionCanceledException(e)) => {
                        classify_cancellation(e.cancellation_reasons())
                    }
                    _ => StoreError::Unavailable,
                })
            }
        }
    }
}

fn classify_cancellation(reasons: &[aws_sdk_dynamodb::types::CancellationReason]) -> StoreError {
    // Transaction item zero is immutable object creation; one and two are quotas.
    if reasons
        .iter()
        .skip(1)
        .any(|r| r.code() == Some("ConditionalCheckFailed"))
    {
        StoreError::Invalid("daily media upload limit reached")
    } else if reasons.iter().any(|r| {
        matches!(
            r.code(),
            Some("ConditionalCheckFailed" | "TransactionConflict")
        )
    }) {
        StoreError::Conflict
    } else {
        StoreError::Unavailable
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_dynamodb::types::CancellationReason as C;
    #[test]
    fn errors_distinguish_quota_conflict_and_storage_failure() {
        assert!(matches!(
            classify_cancellation(&[
                C::builder().code("None").build(),
                C::builder().code("ConditionalCheckFailed").build()
            ]),
            StoreError::Invalid(_)
        ));
        assert!(matches!(
            classify_cancellation(&[C::builder().code("ConditionalCheckFailed").build()]),
            StoreError::Conflict
        ));
        assert!(matches!(
            classify_cancellation(&[C::builder().code("ProvisionedThroughputExceeded").build()]),
            StoreError::Unavailable
        ));
        assert!(matches!(
            classify_cancellation(&[]),
            StoreError::Unavailable
        ));
    }
}
