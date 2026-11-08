//! Portable read-only history archives; live identity and ratchet state stay native.
use super::*;
use croaker_client::backup::{self, Archive, RecoveryCode};
use std::collections::BTreeSet;

fn ensure_confirmation(code: &str, confirmation: &str) -> Result<()> {
    ensure!(
        RecoveryCode::parse(code)?.display().as_str()
            == RecoveryCode::parse(confirmation)?.display().as_str(),
        "Recovery codes do not match. Save the code and enter it again."
    );
    Ok(())
}

fn validate_app_content(archive: &Archive) -> Result<()> {
    for chat in &archive.conversations {
        for message in &chat.messages {
            Content {
                version: 1,
                text: if message.media_omitted.is_some() && message.text.trim().is_empty() {
                    "Attachment omitted".into()
                } else {
                    message.text.clone()
                },
                attachment: message.attachment.clone().map(|file| Attachment {
                    name: file.name,
                    mime: file.mime,
                    data: file.data,
                    size: file.size,
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
    Ok(())
}

impl DeviceRuntime {
    fn export_archive(&mut self) -> Result<Archive> {
        let profile = self.profile()?;
        let source_device = profile.binding.device_id;
        let tenant_id = profile.binding.tenant_id;
        let chats: Vec<_> = profile.chats.values().cloned().collect();
        let mut used = BTreeSet::from([source_device]);
        let mut conversations = Vec::new();
        for chat in chats {
            let mut members = chat.members.clone();
            members.push(source_device);
            let history = self
                .client()?
                .history(chat.id)?
                .into_iter()
                .filter(|message| !message.pending)
                .collect();
            let mut messages = Vec::new();
            for projected in project(history)? {
                members.push(projected.message.sender);
                members.extend(projected.reactions.keys().copied());
                if let Some(reply) = &projected.reply_to {
                    members.push(reply.sender);
                }
                messages.push(backup::Message {
                    id: projected.message.message_id,
                    sender: projected.message.sender,
                    sent_at: projected.message.sent_at,
                    text: projected.content.text,
                    media_omitted: projected.media.map(|media| backup::OmittedMedia {
                        name: media.name,
                        mime: media.mime,
                        size: media.size,
                    }),
                    attachment: projected.content.attachment.map(|file| backup::Attachment {
                        name: file.name,
                        mime: file.mime,
                        data: file.data,
                        size: file.size,
                    }),
                    reactions: projected
                        .reactions
                        .into_iter()
                        .map(|(sender, (emoji, _))| backup::Reaction { sender, emoji })
                        .collect(),
                    reply_to: projected.reply_to.map(|target| backup::MessageRef {
                        message_id: target.message_id,
                        sender: target.sender,
                    }),
                });
            }
            members.sort();
            members.dedup();
            used.extend(members.iter().copied());
            conversations.push(backup::Conversation {
                id: chat.id,
                title: chat.title,
                kind: chat.kind,
                members,
                messages,
            });
        }
        let profile = self.profile()?;
        let people = used
            .into_iter()
            .map(|device_id| backup::Person {
                device_id,
                name: if device_id == source_device {
                    profile.name.clone()
                } else {
                    profile
                        .contacts
                        .get(&device_id)
                        .map(|contact| contact.name.clone())
                        .unwrap_or_else(|| device_id.to_string())
                },
            })
            .collect();
        Ok(Archive {
            version: 1,
            id: MessageId::new(),
            created_at: now(),
            tenant_id,
            source_device,
            people,
            conversations,
        })
    }

    pub(super) fn backups(&mut self, method: &str, route: &str, body: &Value) -> Result<Value> {
        match (method, route) {
            ("POST", "backups/code") => {
                let code = RecoveryCode::generate().display();
                Ok(json!({"recovery_code":code.as_str()}))
            }
            ("POST", "backups/export") => {
                let code = body["recovery_code"]
                    .as_str()
                    .context("Enter the recovery code.")?;
                ensure_confirmation(
                    code,
                    body["confirmation"]
                        .as_str()
                        .context("Confirm the recovery code.")?,
                )?;
                let archive = self.export_archive()?;
                let bytes = backup::encrypt(&archive, &RecoveryCode::parse(code)?)?;
                Ok(
                    json!({"binary_base64":BASE64_STANDARD.encode(bytes),"content_type":"application/octet-stream","filename":"cirra-history.crkbackup"}),
                )
            }
            ("POST", "backups/restore") => {
                let encoded = body["backup"].as_str().context("Choose a backup file.")?;
                ensure!(
                    encoded.len() <= backup::MAX_BACKUP_BYTES.div_ceil(3) * 4,
                    "Backup file is too large."
                );
                let bytes = BASE64_STANDARD
                    .decode(encoded)
                    .context("Invalid backup file encoding.")?;
                let archive = backup::decrypt(
                    &bytes,
                    &RecoveryCode::parse(
                        body["recovery_code"]
                            .as_str()
                            .context("Enter the recovery code.")?,
                    )?,
                )?;
                validate_app_content(&archive)?;
                let imported = self.client()?.import_archive(&archive)?;
                Ok(json!({"id":archive.id,"imported":imported}))
            }
            ("GET", "backups/archives") => {
                let archives = self.client()?.recovered_archives()?;
                Ok(
                    json!({"archives":archives.iter().map(|archive|json!({"id":archive.id,"created_at":archive.created_at,"owner":archive.people.iter().find(|person|person.device_id==archive.source_device).map(|person|person.name.as_str()).unwrap_or("Unknown"),"conversations":archive.conversations.len(),"messages":archive.conversations.iter().map(|chat|chat.messages.len()).sum::<usize>()})).collect::<Vec<_>>()}),
                )
            }
            _ => {
                let archive_route = route
                    .strip_prefix("backups/archives/")
                    .context("Unsupported backup route")?;
                if let Some(id) = archive_route.strip_suffix("/remove") {
                    ensure!(method == "POST", "Unsupported archive method");
                    self.client()?.delete_archive(id.parse::<MessageId>()?)?;
                    Ok(json!({}))
                } else {
                    ensure!(method == "GET", "Unsupported archive method");
                    let id: MessageId = archive_route.parse()?;
                    let archive = self
                        .client()?
                        .recovered_archives()?
                        .into_iter()
                        .find(|archive| archive.id == id)
                        .context("Archive not available to this device.")?;
                    Ok(serde_json::to_value(archive)?)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_export_requires_the_same_confirmed_code() {
        let first = RecoveryCode::generate().display();
        let second = RecoveryCode::generate().display();
        assert!(ensure_confirmation(&first, &first.to_uppercase()).is_ok());
        assert!(ensure_confirmation(&first, &second).is_err());
        assert!(ensure_confirmation(&first, "invalid").is_err());
    }
}
