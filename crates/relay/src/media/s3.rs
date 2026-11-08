//! Immutable S3 objects with 60-second grants and signed size/checksum headers.
use super::*;
use aws_sdk_s3::{Client, presigning::PresigningConfig, types::ChecksumMode};
pub struct S3Objects {
    client: Client,
    bucket: String,
}
impl S3Objects {
    pub fn new(client: Client, bucket: String) -> Self {
        Self { client, bucket }
    }
}
impl S3Objects {
    async fn put_transfer(&self, r: &Reservation) -> Result<Transfer> {
        let p = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(r.key())
            .content_type("application/octet-stream")
            .content_length(r.upload.ciphertext_size as i64)
            .checksum_sha256(checksum(&r.upload.sha256))
            .if_none_match("*")
            .presigned(
                PresigningConfig::expires_in(std::time::Duration::from_secs(60))
                    .map_err(|_| StoreError::Unavailable)?,
            )
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok(Transfer {
            url: p.uri().into(),
            headers: p.headers().map(|(k, v)| (k.into(), v.into())).collect(),
            expires_at: now() + 60,
            requires_device_auth: false,
        })
    }
}
fn checksum(hex: &str) -> String {
    use base64::Engine;
    base64::prelude::BASE64_STANDARD.encode(hex::decode(hex).unwrap_or_default())
}
#[async_trait]
impl Objects for S3Objects {
    async fn upload(&self, r: &Reservation) -> Result<UploadGrant> {
        let head = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(r.key())
            .checksum_mode(ChecksumMode::Enabled)
            .send()
            .await;
        match head {
            Ok(h) => {
                if h.content_length() != Some(r.upload.ciphertext_size as i64)
                    || h.checksum_sha256() != Some(checksum(&r.upload.sha256).as_str())
                {
                    return Err(StoreError::Conflict);
                }
                return Ok(UploadGrant {
                    transfer: None,
                    already_uploaded: true,
                });
            }
            Err(e) => {
                if !e.as_service_error().is_some_and(|e| e.is_not_found()) {
                    return Err(StoreError::Unavailable);
                }
            }
        }
        Ok(UploadGrant {
            transfer: Some(self.put_transfer(r).await?),
            already_uploaded: false,
        })
    }
    async fn download(&self, r: &Reservation) -> Result<Transfer> {
        let p = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(r.key())
            .response_content_type("application/octet-stream")
            .response_content_disposition("attachment")
            .presigned(
                PresigningConfig::expires_in(std::time::Duration::from_secs(60))
                    .map_err(|_| StoreError::Unavailable)?,
            )
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok(Transfer {
            url: p.uri().into(),
            headers: p.headers().map(|(k, v)| (k.into(), v.into())).collect(),
            expires_at: now() + 60,
            requires_device_auth: false,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn put_grant_signs_immutable_bounded_ciphertext_without_network() {
        use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .credentials_provider(Credentials::new(
                "test-access",
                "test-secret",
                None,
                None,
                "test",
            ))
            .build();
        let s = S3Objects::new(Client::from_conf(config), "cirra-test-media".into());
        let r = Reservation {
            tenant: TenantId::new(),
            conversation: ConversationId::new(),
            uploader: DeviceId::new(),
            upload: ObjectUpload {
                object_id: MessageId::new(),
                ciphertext_size: 65536,
                sha256: "0".repeat(64),
            },
            expires_at: now() + RETENTION_SECONDS,
        };
        let t = s.put_transfer(&r).await.unwrap();
        assert!(!t.requires_device_auth);
        assert_eq!(
            t.headers.get("if-none-match").map(String::as_str),
            Some("*")
        );
        assert_eq!(
            t.headers.get("content-length").map(String::as_str),
            Some("65536")
        );
        assert_eq!(
            t.headers.get("content-type").map(String::as_str),
            Some("application/octet-stream")
        );
        assert!(t.url.contains("X-Amz-Expires=60"));
        assert!(t.url.contains("content-length"));
        assert!(t.url.contains("if-none-match"));
        assert!(
            t.url.contains("x-amz-checksum-sha256")
                || t.headers.contains_key("x-amz-checksum-sha256")
        );
        assert!(!t.headers.contains_key("authorization"));
    }
}
