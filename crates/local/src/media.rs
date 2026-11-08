//! Loopback harness bridge; production clients encrypt/decrypt on their own device.
use super::*;
use croaker_client::application::{MediaEvent, MediaKind};
use croaker_crypto::media::decrypt;
use croaker_relay::media::{
    Objects, Registry,
    local::{Files, SqliteRegistry},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaInput {
    pub conversation_id: ConversationId,
    pub request_id: MessageId,
    pub name: String,
    pub mime: String,
    pub data: String,
    pub text: String,
    pub reply_to: Option<ReplyInput>,
}
pub struct MediaFile<'a> {
    pub name: &'a str,
    pub mime: &'a str,
    pub bytes: &'a [u8],
}
impl Workspace {
    pub async fn send_media(
        &mut self,
        alias: &str,
        conversation: ConversationId,
        request_id: MessageId,
        file: MediaFile<'_>,
        text: &str,
        reply_to: Option<(MessageId, String)>,
    ) -> Result<()> {
        let MediaFile { name, mime, bytes } = file;
        ensure!(
            text.len() <= 8000,
            "Messages can contain up to 8,000 bytes of text."
        );
        self.member_chat(alias, conversation)?;
        self.sync_profile(alias).await?;
        let profile = self.profile(alias)?;
        let reply_to = if let Some((id, alias)) = reply_to {
            let sender = self.profile(&alias)?.principal.device_id;
            ensure!(
                events::project(self.clients[&profile.alias].history(conversation)?)?
                    .iter()
                    .any(|p| p.message.message_id == id
                        && p.message.sender == sender
                        && !p.message.pending),
                "Reply target is unavailable."
            );
            Some(MessageRef {
                message_id: id,
                sender,
            })
        } else {
            None
        };
        let prepared = self
            .clients
            .get_mut(alias)
            .context("Unknown account")?
            .prepare_media(request_id, conversation, name, mime, bytes)?;
        // Authenticate against relay membership even though this harness shares its filesystem.
        self.relay
            .conversation_info(&profile.principal, conversation)
            .await?;
        let registry = SqliteRegistry::open(self.directory.join("media.sqlite"))?;
        let files = Files::open(self.directory.join("objects"), String::new())?;
        let record = registry
            .reserve(
                &profile.principal,
                conversation,
                prepared.descriptor.upload(),
            )
            .await?;
        let grant = files.upload(&record).await?;
        if !grant.already_uploaded {
            ensure!(
                !prepared.ciphertext.is_empty(),
                "Uploaded file has expired; choose a new attachment request."
            );
            files.put(&record, &prepared.ciphertext).await?;
        }
        self.clients
            .get_mut(alias)
            .context("Unknown account")?
            .mark_media_uploaded(request_id)?;
        let event = MediaEvent {
            version: 1,
            kind: MediaKind::Media,
            text: text.into(),
            attachment: prepared.descriptor,
            reply_to,
        };
        event.validate()?;
        let payload = Zeroizing::new(serde_json::to_vec(&event)?);
        self.clients
            .get_mut(alias)
            .context("Unknown account")?
            .queue_message_once(conversation, request_id, &payload)?;
        self.flush(alias).await?;
        let _ = self.publish_typing(alias, conversation, false).await;
        Ok(())
    }
    pub async fn download_media(
        &mut self,
        alias: &str,
        conversation: ConversationId,
        message_id: MessageId,
        sender_alias: &str,
    ) -> Result<(String, String, Zeroizing<Vec<u8>>)> {
        self.member_chat(alias, conversation)?;
        let profile = self.profile(alias)?;
        let sender = self.profile(sender_alias)?.principal.device_id;
        let message = events::project(self.clients[alias].history(conversation)?)?
            .into_iter()
            .find(|m| m.message.message_id == message_id && m.message.sender == sender)
            .context("Attachment unavailable")?;
        let descriptor = message.media.context("Attachment unavailable")?;
        self.relay
            .conversation_info(&profile.principal, conversation)
            .await?;
        let registry = SqliteRegistry::open(self.directory.join("media.sqlite"))?;
        let record = registry
            .find(
                profile.principal.tenant_id,
                conversation,
                descriptor.object_id,
            )
            .await?;
        ensure!(
            record.upload == descriptor.upload(),
            "Attachment metadata mismatch."
        );
        let files = Files::open(self.directory.join("objects"), String::new())?;
        let ciphertext = files.read(&record).await?;
        let plain = decrypt(&descriptor, &ciphertext)?;
        Ok((descriptor.name, descriptor.mime, plain))
    }
}
async fn send(
    State(s): State<Shared>,
    RoutePath(alias): RoutePath<String>,
    Json(i): Json<MediaInput>,
) -> std::result::Result<StatusCode, LocalError> {
    ensure_local_size(&i.data)?;
    let b = Zeroizing::new(
        BASE64_STANDARD
            .decode(&i.data)
            .context("Invalid attachment encoding.")?,
    );
    s.lock()
        .await
        .send_media(
            &alias,
            i.conversation_id,
            i.request_id,
            MediaFile {
                name: &i.name,
                mime: &i.mime,
                bytes: &b,
            },
            &i.text,
            i.reply_to.map(|r| (r.message_id, r.sender)),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
fn ensure_local_size(data: &str) -> std::result::Result<(), LocalError> {
    if data.len() > croaker_protocol::media::MAX_MEDIA_BYTES.div_ceil(3) * 4 {
        return Err(LocalError(anyhow::anyhow!("Choose a file up to 25 MiB.")));
    }
    Ok(())
}
#[derive(Deserialize)]
struct DownloadQuery {
    sender: String,
}
async fn download(
    State(s): State<Shared>,
    RoutePath((alias, c, m)): RoutePath<(String, ConversationId, MessageId)>,
    Query(q): Query<DownloadQuery>,
) -> std::result::Result<Response, LocalError> {
    let (_name, mime, b) = s
        .lock()
        .await
        .download_media(&alias, c, m, &q.sender)
        .await?;
    let preview = matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    );
    let mut response = b.to_vec().into_response();
    response.headers_mut().insert(
        "content-type",
        if preview {
            mime.parse().unwrap()
        } else {
            "application/octet-stream".parse().unwrap()
        },
    );
    response
        .headers_mut()
        .insert("content-disposition", "attachment".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    Ok(response)
}
pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route(
            "/{profile}/media/send",
            post(send).layer(axum::extract::DefaultBodyLimit::max(36 * 1024 * 1024)),
        )
        .route("/{profile}/media/{conversation}/{message}", get(download))
}
