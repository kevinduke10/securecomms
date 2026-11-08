//! Authorized ciphertext object storage. Filenames and decryption keys never enter this module.
use crate::{RETENTION_SECONDS, Result, Store, StoreError, now};
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use croaker_protocol::{media::*, *};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reservation {
    pub tenant: TenantId,
    pub conversation: ConversationId,
    pub uploader: DeviceId,
    pub upload: ObjectUpload,
    pub expires_at: u64,
}
impl Reservation {
    pub fn key(&self) -> String {
        format!(
            "{}/{}/{}",
            self.tenant, self.conversation, self.upload.object_id
        )
    }
}
#[async_trait]
pub trait Registry: Send + Sync {
    async fn reserve(
        &self,
        actor: &Principal,
        conversation: ConversationId,
        upload: ObjectUpload,
    ) -> Result<Reservation>;
    async fn find(
        &self,
        tenant: TenantId,
        conversation: ConversationId,
        object: MessageId,
    ) -> Result<Reservation>;
}
#[async_trait]
pub trait Objects: Send + Sync {
    async fn upload(&self, record: &Reservation) -> Result<UploadGrant>;
    async fn download(&self, record: &Reservation) -> Result<Transfer>;
    async fn put(&self, _record: &Reservation, _bytes: &[u8]) -> Result<()> {
        Err(StoreError::NotFound)
    }
    async fn read(&self, _record: &Reservation) -> Result<Vec<u8>> {
        Err(StoreError::NotFound)
    }
}
pub struct ObjectService {
    pub relay: Arc<dyn Store>,
    pub registry: Arc<dyn Registry>,
    pub objects: Arc<dyn Objects>,
}
impl ObjectService {
    pub async fn reserve(
        &self,
        actor: &Principal,
        conversation: ConversationId,
        upload: ObjectUpload,
    ) -> Result<Reservation> {
        upload.validate().map_err(StoreError::Invalid)?;
        self.relay.conversation_info(actor, conversation).await?;
        self.registry.reserve(actor, conversation, upload).await
    }
    pub async fn find(
        &self,
        actor: &Principal,
        conversation: ConversationId,
        object: MessageId,
    ) -> Result<Reservation> {
        self.relay.conversation_info(actor, conversation).await?;
        let r = self
            .registry
            .find(actor.tenant_id, conversation, object)
            .await?;
        if r.expires_at <= now() {
            return Err(StoreError::NotFound);
        }
        Ok(r)
    }
    pub async fn put(
        &self,
        actor: &Principal,
        conversation: ConversationId,
        upload: ObjectUpload,
        bytes: &[u8],
    ) -> Result<()> {
        let r = self.reserve(actor, conversation, upload).await?;
        self.objects.put(&r, bytes).await
    }
    pub async fn read(
        &self,
        actor: &Principal,
        conversation: ConversationId,
        object: MessageId,
    ) -> Result<Vec<u8>> {
        let r = self.find(actor, conversation, object).await?;
        self.objects.read(&r).await
    }
}
pub fn router(service: Arc<ObjectService>) -> Router {
    Router::new()
        .route(
            "/v1/conversations/{conversation}/objects/uploads",
            post(upload),
        )
        .route(
            "/v1/conversations/{conversation}/objects/{object}",
            get(download),
        )
        .layer(DefaultBodyLimit::max(4096))
        .layer(axum::middleware::map_response(private_response))
        .with_state(service)
}
/// Local relay only. Large ciphertext never passes through API Gateway/Lambda.
pub fn local_content_router(service: Arc<ObjectService>) -> Router {
    Router::new()
        .route(
            "/v1/conversations/{conversation}/objects/{object}/content",
            get(read).put(put),
        )
        .layer(DefaultBodyLimit::max(MAX_MEDIA_BYTES + MEDIA_OVERHEAD))
        .layer(axum::middleware::map_response(private_response))
        .with_state(service)
}
async fn private_response(mut response: axum::response::Response) -> axum::response::Response {
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
}
async fn actor(headers: &HeaderMap, s: &ObjectService) -> Result<Principal> {
    let t = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or(StoreError::Unauthorized)?;
    s.relay.authenticate(t).await
}
async fn upload(
    State(s): State<Arc<ObjectService>>,
    headers: HeaderMap,
    Path(c): Path<ConversationId>,
    Json(u): Json<ObjectUpload>,
) -> Result<Json<UploadGrant>> {
    let a = actor(&headers, &s).await?;
    let r = s.reserve(&a, c, u).await?;
    Ok(Json(s.objects.upload(&r).await?))
}
async fn download(
    State(s): State<Arc<ObjectService>>,
    headers: HeaderMap,
    Path((c, o)): Path<(ConversationId, MessageId)>,
) -> Result<Json<Transfer>> {
    let a = actor(&headers, &s).await?;
    let r = s.find(&a, c, o).await?;
    Ok(Json(s.objects.download(&r).await?))
}
async fn put(
    State(s): State<Arc<ObjectService>>,
    headers: HeaderMap,
    Path((c, o)): Path<(ConversationId, MessageId)>,
    bytes: axum::body::Bytes,
) -> Result<axum::http::StatusCode> {
    let a = actor(&headers, &s).await?;
    let r = s.find(&a, c, o).await?;
    if r.uploader != a.device_id {
        return Err(StoreError::NotFound);
    }
    s.objects.put(&r, &bytes).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
async fn read(
    State(s): State<Arc<ObjectService>>,
    headers: HeaderMap,
    Path((c, o)): Path<(ConversationId, MessageId)>,
) -> Result<impl axum::response::IntoResponse> {
    let a = actor(&headers, &s).await?;
    let b = s.read(&a, c, o).await?;
    Ok((
        [
            ("content-type", "application/octet-stream"),
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
        ],
        b,
    ))
}
fn verify(r: &Reservation, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != r.upload.ciphertext_size
        || hex::encode(Sha256::digest(bytes)) != r.upload.sha256
    {
        return Err(StoreError::Invalid("ciphertext does not match reservation"));
    }
    Ok(())
}
#[cfg(feature = "local")]
pub mod local {
    use super::*;
    use rusqlite::{Connection, OptionalExtension, params};
    use std::{
        io::{Read, Write},
        path::{Path, PathBuf},
        sync::Mutex,
    };
    pub struct SqliteRegistry(Mutex<Connection>);
    impl SqliteRegistry {
        pub fn open(path: impl AsRef<Path>) -> Result<Self> {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .mode(0o600)
                    .open(&path)
                    .map_err(|_| StoreError::Unavailable)?;
            }
            let c = Connection::open(path).map_err(|_| StoreError::Unavailable)?;
            c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL;CREATE TABLE IF NOT EXISTS media(key TEXT PRIMARY KEY,record TEXT NOT NULL);CREATE TABLE IF NOT EXISTS quota(key TEXT PRIMARY KEY,bytes INTEGER NOT NULL);").map_err(|_|StoreError::Unavailable)?;
            Ok(Self(Mutex::new(c)))
        }
    }
    #[async_trait]
    impl Registry for SqliteRegistry {
        async fn reserve(
            &self,
            a: &Principal,
            c: ConversationId,
            u: ObjectUpload,
        ) -> Result<Reservation> {
            u.validate().map_err(StoreError::Invalid)?;
            let r = Reservation {
                tenant: a.tenant_id,
                conversation: c,
                uploader: a.device_id,
                upload: u,
                expires_at: now() + RETENTION_SECONDS,
            };
            let key = r.key();
            let mut db = self.0.lock().map_err(|_| StoreError::Unavailable)?;
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| StoreError::Unavailable)?;
            let old: Option<String> = tx
                .query_row("SELECT record FROM media WHERE key=?1", [&key], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(|_| StoreError::Unavailable)?;
            if let Some(old) = old {
                let old: Reservation =
                    serde_json::from_str(&old).map_err(|_| StoreError::Unavailable)?;
                if old.upload != r.upload || old.uploader != a.device_id {
                    return Err(StoreError::Conflict);
                }
                if old.expires_at <= now() {
                    return Err(StoreError::NotFound);
                }
                return Ok(old);
            }
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
                let used: u64 = tx
                    .query_row("SELECT bytes FROM quota WHERE key=?1", [&key], |r| r.get(0))
                    .optional()
                    .map_err(|_| StoreError::Unavailable)?
                    .unwrap_or(0);
                if used + r.upload.ciphertext_size > limit {
                    return Err(StoreError::Invalid("daily media upload limit reached"));
                }
                tx.execute("INSERT INTO quota VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET bytes=excluded.bytes",params![key,used+r.upload.ciphertext_size]).map_err(|_|StoreError::Unavailable)?;
            }
            tx.execute(
                "INSERT INTO media VALUES(?1,?2)",
                params![
                    key,
                    serde_json::to_string(&r).map_err(|_| StoreError::Unavailable)?
                ],
            )
            .map_err(|_| StoreError::Unavailable)?;
            tx.commit().map_err(|_| StoreError::Unavailable)?;
            Ok(r)
        }
        async fn find(&self, t: TenantId, c: ConversationId, o: MessageId) -> Result<Reservation> {
            let db = self.0.lock().map_err(|_| StoreError::Unavailable)?;
            let s: String = db
                .query_row(
                    "SELECT record FROM media WHERE key=?1",
                    [format!("{t}/{c}/{o}")],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| StoreError::Unavailable)?
                .ok_or(StoreError::NotFound)?;
            let r: Reservation = serde_json::from_str(&s).map_err(|_| StoreError::Unavailable)?;
            if r.expires_at <= now() {
                return Err(StoreError::NotFound);
            }
            Ok(r)
        }
    }
    pub struct Files {
        root: PathBuf,
        base: String,
    }
    impl Files {
        pub fn open(root: impl AsRef<Path>, base: String) -> Result<Self> {
            std::fs::create_dir_all(&root).map_err(|_| StoreError::Unavailable)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                    .map_err(|_| StoreError::Unavailable)?;
            }
            Ok(Self {
                root: root.as_ref().to_path_buf(),
                base,
            })
        }
        fn path(&self, r: &Reservation) -> PathBuf {
            self.root.join(r.key().replace('/', "_"))
        }
        fn transfer(&self, r: &Reservation) -> Transfer {
            Transfer {
                url: format!(
                    "{}/v1/conversations/{}/objects/{}/content",
                    self.base, r.conversation, r.upload.object_id
                ),
                headers: Default::default(),
                expires_at: now() + 60,
                requires_device_auth: true,
            }
        }
    }
    #[async_trait]
    impl Objects for Files {
        async fn upload(&self, r: &Reservation) -> Result<UploadGrant> {
            if self.path(r).exists() {
                self.read(r).await?;
                return Ok(UploadGrant {
                    transfer: None,
                    already_uploaded: true,
                });
            }
            Ok(UploadGrant {
                transfer: Some(self.transfer(r)),
                already_uploaded: false,
            })
        }
        async fn download(&self, r: &Reservation) -> Result<Transfer> {
            self.read(r).await?;
            Ok(self.transfer(r))
        }
        async fn put(&self, r: &Reservation, b: &[u8]) -> Result<()> {
            verify(r, b)?;
            let path = self.path(r);
            if path.exists() {
                return self.read(r).await.and_then(|old| {
                    if old == b {
                        Ok(())
                    } else {
                        Err(StoreError::Conflict)
                    }
                });
            }
            let temp = self.root.join(format!(".{}", MessageId::new()));
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let result = (|| {
                let mut f = opts.open(&temp).map_err(|_| StoreError::Unavailable)?;
                f.write_all(b)
                    .and_then(|_| f.sync_all())
                    .map_err(|_| StoreError::Unavailable)?;
                match std::fs::hard_link(&temp, &path) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                    Err(_) => Err(StoreError::Unavailable),
                }
            })();
            let _ = std::fs::remove_file(temp);
            result?;
            let old = self.read(r).await?;
            if old != b {
                return Err(StoreError::Conflict);
            }
            Ok(())
        }
        async fn read(&self, r: &Reservation) -> Result<Vec<u8>> {
            let f = std::fs::File::open(self.path(r)).map_err(|_| StoreError::NotFound)?;
            let mut b = Vec::new();
            f.take(r.upload.ciphertext_size + 1)
                .read_to_end(&mut b)
                .map_err(|_| StoreError::Unavailable)?;
            verify(r, &b)?;
            Ok(b)
        }
    }
}
#[cfg(feature = "aws")]
pub mod s3;

#[cfg(feature = "aws")]
pub mod dynamodb;
