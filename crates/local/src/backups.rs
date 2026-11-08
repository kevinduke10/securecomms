//! Local client bridge only. Recovery secrets and plaintext never go to the relay.
use super::*;
use croaker_client::backup::{self, Archive, RecoveryCode};
use zeroize::Zeroize;

impl Workspace {
    pub fn export_backup(&self, alias: &str, code: &str) -> Result<Vec<u8>> {
        let profile = self.profile(alias)?;
        let recovery = RecoveryCode::parse(code)?;
        let mut conversations = Vec::new();
        let mut used = std::collections::HashSet::from([profile.principal.device_id]);
        for chat in &self.manifest.chats {
            if !chat.members.iter().any(|m| m == alias) {
                continue;
            }
            let mut members = chat
                .members
                .iter()
                .map(|alias| self.profile(alias).map(|p| p.principal.device_id))
                .collect::<Result<Vec<_>>>()?;
            used.extend(members.iter().copied());
            // Saved, delivered history only. Pending messages and reaction changes
            // are excluded rather than converting an unsent action into history.
            let history = self.clients[alias]
                .history(chat.id)?
                .into_iter()
                .filter(|m| !m.pending)
                .collect();
            let mut messages = Vec::new();
            for projected in events::project(history)? {
                let message = projected.message;
                let content = projected.content;
                messages.push(backup::Message {
                    reply_to: projected.reply_to.map(|target| backup::MessageRef {
                        message_id: target.message_id,
                        sender: target.sender,
                    }),
                    id: message.message_id,
                    sender: message.sender,
                    sent_at: message.sent_at,
                    text: content.text,
                    media_omitted: projected.media.map(|media| backup::OmittedMedia {
                        name: media.name,
                        mime: media.mime,
                        size: media.size,
                    }),
                    attachment: content.attachment.map(|f| backup::Attachment {
                        name: f.name,
                        mime: f.mime,
                        data: f.data,
                        size: f.size,
                    }),
                    reactions: projected
                        .reactions
                        .into_iter()
                        .map(|(sender, (emoji, _))| backup::Reaction { sender, emoji })
                        .collect(),
                });
            }
            // Backup participants describe historical authors, not present access.
            for message in &messages {
                members.push(message.sender);
                members.extend(message.reactions.iter().map(|r| r.sender));
                if let Some(reply) = &message.reply_to {
                    members.push(reply.sender);
                }
            }
            members.sort();
            members.dedup();
            used.extend(members.iter().copied());
            let title = if chat.kind == "direct" {
                self.profile(
                    chat.members
                        .iter()
                        .find(|m| m.as_str() != alias)
                        .context("Missing direct participant")?,
                )?
                .name
            } else {
                chat.title.clone()
            };
            conversations.push(backup::Conversation {
                id: chat.id,
                title,
                kind: chat.kind.clone(),
                members,
                messages,
            });
        }
        let people = self
            .manifest
            .profiles
            .iter()
            .filter(|p| used.contains(&p.principal.device_id))
            .map(|p| backup::Person {
                device_id: p.principal.device_id,
                name: p.name.clone(),
            })
            .collect();
        let archive = Archive {
            version: 1,
            id: MessageId::new(),
            created_at: croaker_relay::now(),
            tenant_id: profile.principal.tenant_id,
            source_device: profile.principal.device_id,
            people,
            conversations,
        };
        Ok(backup::encrypt(&archive, &recovery)?)
    }

    pub fn restore_backup(
        &mut self,
        alias: &str,
        code: &str,
        bytes: &[u8],
    ) -> Result<(MessageId, bool)> {
        self.profile(alias)?;
        let archive = backup::decrypt(bytes, &RecoveryCode::parse(code)?)?;
        // Validate app payloads too; no archive content becomes executable markup,
        // messaging authority, a key binding, or an outgoing event.
        for chat in &archive.conversations {
            for message in &chat.messages {
                Content {
                    version: 1,
                    text: if message.media_omitted.is_some() && message.text.trim().is_empty() {
                        "Attachment omitted".into()
                    } else {
                        message.text.clone()
                    },
                    attachment: message.attachment.clone().map(|f| Attachment {
                        name: f.name,
                        mime: f.mime,
                        data: f.data,
                        size: f.size,
                    }),
                }
                .validate()?;
                for reaction in &message.reactions {
                    Reaction {
                        version: 1,
                        kind: ReactionKind::Reaction,
                        target_message_id: message.id,
                        target_sender: message.sender,
                        emoji: Some(reaction.emoji.clone()),
                    }
                    .validate()?;
                }
            }
        }
        let imported = self
            .clients
            .get_mut(alias)
            .unwrap()
            .import_archive(&archive)?;
        Ok((archive.id, imported))
    }

    pub fn backup_archives(&self, alias: &str) -> Result<Vec<Archive>> {
        self.profile(alias)?;
        Ok(self.clients[alias].recovered_archives()?)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportInput {
    recovery_code: String,
    confirmation: String,
}
impl Drop for ExportInput {
    fn drop(&mut self) {
        self.recovery_code.zeroize();
        self.confirmation.zeroize();
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreInput {
    recovery_code: String,
    backup: String,
}
impl Drop for RestoreInput {
    fn drop(&mut self) {
        self.recovery_code.zeroize();
    }
}

async fn generate(
    State(workspace): State<Shared>,
    RoutePath(alias): RoutePath<String>,
) -> std::result::Result<Json<serde_json::Value>, LocalError> {
    workspace.lock().await.profile(&alias)?;
    let code = RecoveryCode::generate().display();
    Ok(Json(serde_json::json!({"recovery_code":code.as_str()})))
}
async fn export(
    State(workspace): State<Shared>,
    RoutePath(alias): RoutePath<String>,
    Json(input): Json<ExportInput>,
) -> std::result::Result<Response, LocalError> {
    ensure_confirmation(&input.recovery_code, &input.confirmation)?;
    let bytes = workspace
        .lock()
        .await
        .export_backup(&alias, &input.recovery_code)?;
    Ok((
        [
            ("content-type", "application/octet-stream"),
            (
                "content-disposition",
                "attachment; filename=\"cirra-history.crkbackup\"",
            ),
        ],
        bytes,
    )
        .into_response())
}
fn ensure_confirmation(code: &str, confirmation: &str) -> Result<()> {
    let first = RecoveryCode::parse(code)?.display();
    let second = RecoveryCode::parse(confirmation)?.display();
    ensure!(
        first.as_str() == second.as_str(),
        "Recovery codes do not match. Save the code and enter it again."
    );
    Ok(())
}
async fn restore(
    State(workspace): State<Shared>,
    RoutePath(alias): RoutePath<String>,
    Json(input): Json<RestoreInput>,
) -> std::result::Result<Json<serde_json::Value>, LocalError> {
    if input.backup.len() > backup::MAX_BACKUP_BYTES.div_ceil(3) * 4 {
        return Err(anyhow::anyhow!("Backup file is too large.").into());
    }
    let bytes = BASE64_STANDARD
        .decode(&input.backup)
        .context("Invalid backup file encoding.")?;
    let (id, imported) =
        workspace
            .lock()
            .await
            .restore_backup(&alias, &input.recovery_code, &bytes)?;
    Ok(Json(serde_json::json!({"id":id,"imported":imported})))
}
async fn list(
    State(workspace): State<Shared>,
    RoutePath(alias): RoutePath<String>,
) -> std::result::Result<Json<serde_json::Value>, LocalError> {
    let archives = workspace.lock().await.backup_archives(&alias)?;
    Ok(Json(
        serde_json::json!({"archives":archives.iter().map(|a|serde_json::json!({"id":a.id,"created_at":a.created_at,"owner":a.people.iter().find(|p|p.device_id==a.source_device).map(|p|p.name.as_str()).unwrap_or("Unknown"),"conversations":a.conversations.len(),"messages":a.conversations.iter().map(|c|c.messages.len()).sum::<usize>()})).collect::<Vec<_>>()}),
    ))
}
async fn read(
    State(workspace): State<Shared>,
    RoutePath((alias, id)): RoutePath<(String, MessageId)>,
) -> std::result::Result<Json<Archive>, LocalError> {
    let archive = workspace
        .lock()
        .await
        .backup_archives(&alias)?
        .into_iter()
        .find(|a| a.id == id)
        .context("Archive not available to this account.")?;
    Ok(Json(archive))
}
async fn remove(
    State(workspace): State<Shared>,
    RoutePath((alias, id)): RoutePath<(String, MessageId)>,
) -> std::result::Result<StatusCode, LocalError> {
    let mut workspace = workspace.lock().await;
    workspace.profile(&alias)?;
    workspace
        .clients
        .get_mut(&alias)
        .unwrap()
        .delete_archive(id)
        .map_err(anyhow::Error::from)?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/{profile}/backups/code", post(generate))
        .route(
            "/{profile}/backups/export",
            post(export).layer(DefaultBodyLimit::max(1024)),
        )
        .route(
            "/{profile}/backups/restore",
            post(restore).layer(DefaultBodyLimit::max(
                backup::MAX_BACKUP_BYTES.div_ceil(3) * 4 + 1024,
            )),
        )
        .route("/{profile}/backups/archives", get(list))
        .route("/{profile}/backups/archives/{id}", get(read))
        .route("/{profile}/backups/archives/{id}/remove", post(remove))
}
