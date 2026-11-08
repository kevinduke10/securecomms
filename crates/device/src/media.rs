use super::*;
use croaker_protocol::media::{MAX_MEDIA_BYTES, MEDIA_OVERHEAD, Transfer, UploadGrant};
impl DeviceRuntime {
    async fn transfer(
        &mut self,
        transfer: &Transfer,
        bytes: Option<Vec<u8>>,
        limit: usize,
    ) -> Result<Vec<u8>> {
        ensure!(
            transfer.expires_at >= now() && transfer.expires_at <= now() + 120,
            "Expired media authorization"
        );
        let url = reqwest::Url::parse(&transfer.url)?;
        ensure!(
            url.username().is_empty() && url.password().is_none() && url.fragment().is_none(),
            "Invalid transfer URL"
        );
        let mut request = if bytes.is_some() {
            self.http.put(url.clone())
        } else {
            self.http.get(url.clone())
        };
        if transfer.requires_device_auth {
            let base = checked_url(&self.connection.as_ref().context("Missing relay")?.relay)?;
            ensure!(
                url.origin() == base.origin()
                    && url.path().starts_with(&format!(
                        "{}/v1/conversations/",
                        base.path().trim_end_matches('/')
                    ))
                    && url.path().ends_with("/content")
                    && url.query().is_none(),
                "Refusing to send a device credential to another host"
            );
            request = request.bearer_auth(&self.profile()?.token);
        } else {
            ensure!(
                url.scheme() == "https" && url.host_str().is_some(),
                "Encrypted object transfers require HTTPS"
            );
        }
        for (name, value) in &transfer.headers {
            ensure!(
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    "content-type" | "content-length" | "x-amz-checksum-sha256" | "if-none-match"
                ),
                "Invalid object transfer header"
            );
            request = request.header(name, value);
        }
        if let Some(bytes) = bytes {
            request = request.body(bytes);
        }
        self.checkpoint().await?;
        let response = request
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Could not reach encrypted object storage."))?;
        ensure!(
            response.status().is_success(),
            "Encrypted file transfer failed ({})",
            response.status()
        );
        ensure!(
            response.content_length().is_none_or(|n| n <= limit as u64),
            "Object response exceeds its expected size"
        );
        let result = bounded_response(response, limit).await?;
        Ok(result)
    }
    pub(super) async fn media_request(
        &mut self,
        method: &str,
        route: &str,
        body: &Value,
    ) -> Result<Value> {
        self.prepare_group_intents()?;
        if method == "POST" && route == "media/send" {
            let conversation: ConversationId =
                serde_json::from_value(body["conversation_id"].clone())?;
            ensure!(
                self.profile()?.chats.contains_key(&conversation),
                "Unknown conversation"
            );
            let request: MessageId = serde_json::from_value(body["request_id"].clone())?;
            let name = body["name"].as_str().context("Missing file name")?;
            let mime = body["mime"].as_str().context("Missing file type")?;
            let encoded = body["data"].as_str().context("Missing file")?;
            ensure!(
                encoded.len() <= MAX_MEDIA_BYTES.div_ceil(3) * 4,
                "File exceeds 25 MiB"
            );
            let bytes = Zeroizing::new(BASE64_STANDARD.decode(encoded)?);
            let prepared =
                self.client()?
                    .prepare_media(request, conversation, name, mime, &bytes)?;
            let grant: UploadGrant = serde_json::from_value(
                self.remote(
                    "POST",
                    &format!("/v1/conversations/{conversation}/objects/uploads"),
                    Some(&serde_json::to_value(prepared.descriptor.upload())?),
                    false,
                )
                .await?,
            )?;
            if !grant.already_uploaded {
                ensure!(
                    !prepared.ciphertext.is_empty(),
                    "Uploaded file has expired; choose a new attachment request."
                );
                self.transfer(
                    grant
                        .transfer
                        .as_ref()
                        .context("Missing upload authorization")?,
                    Some(prepared.ciphertext),
                    4096,
                )
                .await?;
            }
            self.client()?.mark_media_uploaded(request)?;
            let event = MediaEvent {
                version: 1,
                kind: MediaKind::Media,
                text: body["text"].as_str().unwrap_or("").to_owned(),
                attachment: prepared.descriptor,
                reply_to: if body["reply_to"].is_object() {
                    Some(serde_json::from_value(body["reply_to"].clone())?)
                } else {
                    None
                },
            };
            event.validate()?;
            if let Some(reference) = &event.reply_to {
                ensure!(
                    project(self.client()?.history(conversation)?)?
                        .iter()
                        .any(|m| m.message.sender == reference.sender
                            && m.message.message_id == reference.message_id
                            && !m.message.pending),
                    "Reply target unavailable"
                );
            }
            self.client()?.queue_message_once(
                conversation,
                request,
                &serde_json::to_vec(&event)?,
            )?;
            let error = self.flush_media().await.err().map(|e| e.to_string());
            return Ok(json!({"queued":true,"sync_error":error}));
        }
        if method == "GET"
            && let Some(path) = route.strip_prefix("media/")
        {
            let (ids, sender) = path.split_once("?sender=").context("Missing sender")?;
            let (conversation, message) = ids.split_once('/').context("Invalid file route")?;
            let conversation: ConversationId = conversation.parse()?;
            let message: MessageId = message.parse()?;
            let sender: DeviceId = sender.parse()?;
            ensure!(
                self.profile()?.chats.contains_key(&conversation),
                "Unknown conversation"
            );
            let descriptor = project(self.client()?.history(conversation)?)?
                .into_iter()
                .find(|m| m.message.message_id == message && m.message.sender == sender)
                .and_then(|m| m.media)
                .context("File is not in saved authenticated history")?;
            let grant: Transfer = serde_json::from_value(
                self.remote(
                    "GET",
                    &format!(
                        "/v1/conversations/{conversation}/objects/{}",
                        descriptor.object_id
                    ),
                    None,
                    false,
                )
                .await?,
            )?;
            let ciphertext = self
                .transfer(&grant, None, MAX_MEDIA_BYTES + MEDIA_OVERHEAD)
                .await?;
            let bytes = croaker_crypto::media::decrypt(&descriptor, &ciphertext)?;
            return Ok(
                json!({"binary_base64":BASE64_STANDARD.encode(bytes.as_slice()),"content_type":"application/octet-stream","filename":descriptor.name}),
            );
        }
        bail!("Unsupported media route")
    }
    async fn flush_media(&mut self) -> Result<()> {
        self.sync_for_media().await
    }
}
