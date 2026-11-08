//! Durable pre-upload cache: request retries reuse the original key, nonce and object ID.
use crate::*;
use croaker_crypto::media::{PreparedObject, encrypt};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
#[derive(Serialize, Deserialize)]
struct Cached {
    descriptor: croaker_protocol::media::ObjectAttachment,
    source_digest: String,
}
impl ClientStore {
    /// Compact only after the transport confirms immutable storage (successful PUT or verified existing object).
    /// The sealed descriptor/fingerprint remains so retries never generate fresh encryption.
    pub fn mark_media_uploaded(&mut self, request_id: MessageId) -> Result<()> {
        let count = self
            .connection
            .execute(
                "UPDATE media_uploads SET ciphertext=X'' WHERE request_id=?1",
                [request_id.to_string()],
            )
            .map_err(|_| ClientError::Storage)?;
        if count != 1 {
            return Err(ClientError::Storage);
        }
        Ok(())
    }
    pub fn prepare_media(
        &mut self,
        request_id: MessageId,
        conversation: ConversationId,
        name: &str,
        mime: &str,
        plaintext: &[u8],
    ) -> Result<PreparedObject> {
        if plaintext.len() > croaker_protocol::media::MAX_MEDIA_BYTES {
            return Err(ClientError::Storage);
        }
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS media_uploads(request_id TEXT PRIMARY KEY, sealed BLOB NOT NULL, ciphertext BLOB NOT NULL, digest TEXT NOT NULL)").map_err(|_|ClientError::Storage)?;
        let context = format!("cirra-media-upload-v1:{request_id}");
        let digest = hex::encode(Sha256::digest(plaintext));
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| ClientError::Storage)?;
        let old: Option<(Vec<u8>, Vec<u8>, String)> = tx
            .query_row(
                "SELECT sealed,ciphertext,digest FROM media_uploads WHERE request_id=?1",
                [request_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| ClientError::Storage)?;
        if let Some((sealed, ciphertext, _)) = old {
            let bytes = croaker_crypto::open_record(&sealed, &self.key, context.as_bytes())?;
            let cached: Cached =
                serde_json::from_slice(&bytes).map_err(|_| ClientError::Storage)?;
            let descriptor = cached.descriptor;
            if cached.source_digest != digest
                || descriptor.conversation_id != conversation
                || descriptor.name != name
                || descriptor.mime != mime
            {
                return Err(ClientError::Storage);
            }
            if !ciphertext.is_empty() {
                croaker_crypto::media::decrypt(&descriptor, &ciphertext)?;
            }
            return Ok(PreparedObject {
                descriptor,
                ciphertext,
            });
        }
        // Bound retained pending uploads independently from history/vault limits.
        let used: u64 = tx
            .query_row(
                "SELECT COALESCE(SUM(length(ciphertext)),0) FROM media_uploads",
                [],
                |r| r.get(0),
            )
            .map_err(|_| ClientError::Storage)?;
        let count: u64 = tx
            .query_row("SELECT COUNT(*) FROM media_uploads", [], |r| r.get(0))
            .map_err(|_| ClientError::Storage)?;
        if count >= 1024
            || used + plaintext.len() as u64 + croaker_protocol::media::MEDIA_OVERHEAD as u64
                > 512 * 1024 * 1024
        {
            return Err(ClientError::MediaCapacity);
        }
        let prepared = encrypt(conversation, name, mime, plaintext)?;
        let bytes = Zeroizing::new(
            serde_json::to_vec(&Cached {
                descriptor: prepared.descriptor.clone(),
                source_digest: digest,
            })
            .map_err(|_| ClientError::Storage)?,
        );
        let sealed = croaker_crypto::seal_record(&bytes, &self.key, context.as_bytes())?;
        tx.execute(
            "INSERT INTO media_uploads VALUES(?1,?2,?3,?4)",
            params![request_id.to_string(), sealed, &prepared.ciphertext, ""],
        )
        .map_err(|_| ClientError::Storage)?;
        tx.commit().map_err(|_| ClientError::Storage)?;
        Ok(prepared)
    }
}
