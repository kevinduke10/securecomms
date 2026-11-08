use super::*;
impl DeviceRuntime {
    pub(super) async fn sync_for_media(&mut self) -> Result<()> {
        self.flush().await
    }

    pub(super) async fn flush(&mut self) -> Result<()> {
        self.prepare_group_intents()?;
        if !self.online {
            return Ok(());
        }
        // Creation intent is durable before MLS generation or any publication.
        let own = self.profile()?.binding.device_id;
        let new = self
            .profile()?
            .chats
            .values()
            .filter(|c| c.owner == own && !c.created)
            .cloned()
            .collect::<Vec<_>>();
        for chat in new {
            if !self.client()?.update(|d| Ok(d.has_conversation(chat.id)))? {
                let packages = chat
                    .members
                    .iter()
                    .filter(|id| **id != own)
                    .map(|id| {
                        let p = self
                            .profile()?
                            .contacts
                            .get(id)
                            .context("Missing verified contact")?;
                        Ok((BASE64_STANDARD.decode(&p.package)?, p.binding.clone()))
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.client()?.create_group(chat.id, &packages)?;
            }
            self.remote(
                "POST",
                "/v1/conversations",
                Some(&json!(CreateConversation {
                    id: chat.id,
                    devices: chat.members
                })),
                false,
            )
            .await?;
            self.private
                .as_mut()
                .unwrap()
                .chats
                .get_mut(&chat.id)
                .unwrap()
                .created = true;
            self.save()?;
        }
        self.flush_group_decisions().await?;
        for pending in self.client()?.pending()? {
            self.remote(
                "POST",
                &format!("/v1/conversations/{}/events", pending.conversation_id),
                Some(&serde_json::to_value(&pending.event)?),
                false,
            )
            .await?;
            self.client()?.acknowledge(pending.event.message_id)?;
        }
        Ok(())
    }
    pub(super) async fn sync(&mut self) -> Result<()> {
        self.prepare_group_intents()?;
        if !self.online {
            return Ok(());
        }
        self.flush().await?;
        let chats = self.profile()?.chats.values().cloned().collect::<Vec<_>>();
        for chat in chats {
            let peers = self.peers(&chat);
            loop {
                let cursor = self.client()?.cursor(chat.id)?;
                let page: SyncPage = serde_json::from_value(
                    self.remote(
                        "GET",
                        &format!(
                            "/v1/conversations/{}/sync?after={cursor}&limit={MAX_SYNC_PAGE}",
                            chat.id
                        ),
                        None,
                        false,
                    )
                    .await?,
                )?;
                let approvals = self.profile()?.approvals.clone();
                let lifecycles = self.profile()?.lifecycles.clone();
                let authority = peers
                    .iter()
                    .find(|p| p.device_id == chat.owner)
                    .context("Missing pinned group owner")?;
                let policy = croaker_client::ReceivePolicy {
                    additions: &approvals,
                    lifecycles: &lifecycles,
                    welcome: Some(croaker_client::WelcomeAuthorization {
                        authority,
                        members: &chat.members,
                    }),
                };
                let applied = self.client()?.apply_page_with_policy_report(
                    chat.id,
                    cursor,
                    &page,
                    &peers,
                    is_transient,
                    policy,
                )?;
                self.reconcile_group_decisions()?;
                for signal in applied.transient {
                    if let Ok(ChatEvent::Typing(event)) =
                        serde_json::from_slice(&signal.message.plaintext)
                    {
                        let expiry = if event.version == 1
                            && event.active
                            && event.expires_at <= now() + TYPING_TTL + 2
                        {
                            event.expires_at
                        } else {
                            0
                        };
                        self.typing.insert((chat.id, signal.message.sender), expiry);
                    }
                }
                if !page.has_more {
                    break;
                }
            }
            self.receipts(chat.id, ReceiptState::Delivered, None)?;
        }
        self.flush().await
    }
    fn receipts(
        &mut self,
        conversation: ConversationId,
        state: ReceiptState,
        targets: Option<Vec<MessageRef>>,
    ) -> Result<()> {
        self.prepare_group_intents()?;
        let own = self.profile()?.binding.device_id;
        let projected = project(self.client()?.history(conversation)?)?;
        let requested = targets.clone();
        let mut selected = projected
            .iter()
            .filter(|m| {
                m.message.sender != own
                    && !m.message.pending
                    && m.receipts.get(&own).is_none_or(|s| *s < state)
            })
            .map(|m| MessageRef {
                message_id: m.message.message_id,
                sender: m.message.sender,
            })
            .filter(|m| requested.as_ref().is_none_or(|r| r.contains(m)))
            .collect::<Vec<_>>();
        if let Some(targets) = targets {
            ensure!(
                targets.len() <= 64
                    && targets
                        .iter()
                        .all(|t| projected.iter().any(|m| m.message.sender == t.sender
                            && m.message.message_id == t.message_id
                            && !m.message.pending
                            && t.sender != own)),
                "Invalid receipt target"
            );
        }
        selected.sort();
        selected.dedup();
        for targets in selected.chunks(64) {
            let bytes = Zeroizing::new(serde_json::to_vec(&Receipt {
                version: 1,
                kind: ReceiptKind::Receipt,
                state,
                targets: targets.to_vec(),
            })?);
            let mut hash = Sha256::new();
            hash.update(b"croaker-receipt-event-v1");
            hash.update(conversation.0.as_bytes());
            hash.update(own.0.as_bytes());
            hash.update(bytes.as_slice());
            let id = hex::encode(hash.finalize());
            let id: MessageId = format!(
                "{}-{}-{}-{}-{}",
                &id[..8],
                &id[8..12],
                &id[12..16],
                &id[16..20],
                &id[20..32]
            )
            .parse()?;
            self.client()?
                .queue_message_once(conversation, id, &bytes)?;
        }
        Ok(())
    }
    pub(super) async fn snapshot(&mut self) -> Result<Value> {
        // Saved history remains available during a network outage; failed envelopes
        // remain in the transactional outbox. Surface the actual sync failure.
        let error = self.sync().await.err().map(|e| e.to_string());
        let p = self.profile()?;
        let own = p.binding.device_id;
        let mut profiles = vec![
            json!({"id":own,"name":p.name,"title":"This device","role":p.role,"online":self.online,"fingerprint":p.binding.fingerprint()}),
        ];
        profiles.extend(p.contacts.values().map(|c|json!({"id":c.binding.device_id,"name":c.name,"title":"Verified device","role":"member","online":false,"fingerprint":c.binding.fingerprint()})));
        let chats = p.chats.values().cloned().collect::<Vec<_>>();
        let mut conversations = Vec::new();
        for chat in chats {
            let history = project(self.client()?.history(chat.id)?)?;
            let sources = history
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    (
                        (m.message.sender, m.message.message_id),
                        (i, m.content.clone()),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let messages=history.into_iter().enumerate().map(|(index,p)|{
                let mut reactions=BTreeMap::<String,(Vec<DeviceId>,bool)>::new();for (sender,(emoji,pending)) in p.reactions {let entry=reactions.entry(emoji).or_default();entry.0.push(sender);entry.1|=pending;}
                let reactions=reactions.into_iter().map(|(emoji,(users,pending))|json!({"emoji":emoji,"users":users,"pending":pending})).collect::<Vec<_>>();
                let reply=p.reply_to.map(|target|{let source=sources.get(&(target.sender,target.message_id)).filter(|(i,_)|*i<index);json!({"id":target.message_id,"sender":target.sender,"available":source.is_some(),"text":source.map(|(_,c)|&c.text),"attachment_name":source.and_then(|(_,c)|c.attachment.as_ref().map(|a|&a.name))})});
                json!({"id":p.message.message_id,"sender":p.message.sender,"sent_at":p.message.sent_at,"pending":p.message.pending,"text":p.content.text,"attachment":p.content.attachment,"media":p.media.as_ref().map(|m|m.summary()),"reactions":reactions,"reply_to":reply,"read_by_me":p.receipts.get(&own)==Some(&ReceiptState::Read),"receipts":{"delivered":p.receipts.iter().filter(|(_,s)|**s>=ReceiptState::Delivered).map(|(d,_)|d).collect::<Vec<_>>(),"read":p.receipts.iter().filter(|(_,s)|**s==ReceiptState::Read).map(|(d,_)|d).collect::<Vec<_>>()}})
            }).collect::<Vec<_>>();
            let typing = self
                .typing
                .iter()
                .filter(|((c, d), expiry)| *c == chat.id && *d != own && **expiry > now())
                .map(|((_, d), _)| d)
                .collect::<Vec<_>>();
            conversations.push(json!({"id":chat.id,"title":chat.title,"kind":chat.kind,"members":chat.members,"owner":chat.owner,"messages":messages,"typing":typing,"unavailable_messages":self.client()?.stale_application_count(chat.id)?}));
        }
        Ok(
            json!({"test_mode":false,"current_profile":own,"profiles":profiles,"conversations":conversations,"online":self.online,"sync_error":error,"attachment_limit":croaker_protocol::media::MAX_MEDIA_BYTES,"read_receipts":self.profile()?.read_receipts,"reaction_options":REACTION_EMOJIS}),
        )
    }
    pub(super) async fn messaging_request(
        &mut self,
        method: &str,
        route: &str,
        body: Value,
    ) -> Result<Value> {
        self.prepare_group_intents()?;
        if let Some(result) = self.group_request(method, route, &body).await? {
            return Ok(result);
        }
        if route.starts_with("backups") {
            return self.backups(method, route, &body);
        }
        if route.starts_with("media/") {
            return self.media_request(method, route, &body).await;
        }
        ensure!(method == "POST", "Unsupported client method");
        if route == "online" {
            self.online = body["online"].as_bool().context("Invalid online setting")?;
            if self.online {
                self.sync().await?;
            }
            return Ok(json!({}));
        }
        if route == "receipts/settings" {
            self.private.as_mut().unwrap().read_receipts = body["enabled"]
                .as_bool()
                .context("Invalid receipt setting")?;
            self.save()?;
            return Ok(json!({}));
        }
        if route == "conversations" {
            ensure!(
                self.profile()?.role != Role::Guest,
                "Guests cannot create conversations"
            );
            let members: Vec<DeviceId> = serde_json::from_value(body["members"].clone())?;
            let kind = body["kind"].as_str().context("Choose conversation type")?;
            ensure!(
                matches!(kind, "direct" | "group")
                    && !members.is_empty()
                    && members.len() < MAX_CONVERSATION_DEVICES
                    && (kind != "direct" || members.len() == 1),
                "Invalid group"
            );
            let title = if kind == "direct" {
                self.profile()?
                    .contacts
                    .get(&members[0])
                    .context("Verify the contact first")?
                    .name
                    .clone()
            } else {
                body["title"]
                    .as_str()
                    .context("Enter group name")?
                    .trim()
                    .to_owned()
            };
            ensure!(
                (2..=80).contains(&title.len()),
                "Enter a group name with 2–80 characters"
            );
            for member in &members {
                ensure!(
                    self.profile()?.contacts.contains_key(member),
                    "Verify every contact before creating a group"
                );
            }
            let own = self.profile()?.binding.device_id;
            let mut members = members;
            members.push(own);
            members.sort();
            members.dedup();
            let id = ConversationId::new();
            self.private.as_mut().unwrap().chats.insert(
                id,
                Chat {
                    id,
                    title,
                    kind: kind.to_owned(),
                    owner: own,
                    members,
                    created: false,
                },
            );
            self.save()?;
            self.flush().await?;
            return Ok(json!({"id":id}));
        }
        let conversation: ConversationId = serde_json::from_value(body["conversation_id"].clone())?;
        ensure!(
            self.profile()?.chats.contains_key(&conversation),
            "Unknown conversation"
        );
        match route {
            "send" => {
                let content: Content = serde_json::from_value(body["content"].clone())?;
                content.validate()?;
                let id: MessageId = serde_json::from_value(body["request_id"].clone())?;
                let bytes = if body["reply_to"].is_object() {
                    let reference: MessageRef = serde_json::from_value(body["reply_to"].clone())?;
                    ensure!(
                        project(self.client()?.history(conversation)?)?
                            .iter()
                            .any(|m| m.message.message_id == reference.message_id
                                && m.message.sender == reference.sender
                                && !m.message.pending),
                        "Reply target unavailable"
                    );
                    serde_json::to_vec(&Reply {
                        version: 1,
                        kind: ReplyKind::Reply,
                        reply_to: reference,
                        content,
                    })?
                } else {
                    serde_json::to_vec(&content)?
                };
                self.client()?
                    .queue_message_once(conversation, id, &bytes)?;
            }
            "reactions" => {
                let reaction: Reaction = serde_json::from_value(
                    json!({"version":1,"kind":"reaction","target_message_id":body["target_message_id"],"target_sender":body["target_sender"],"emoji":body["emoji"]}),
                )?;
                reaction.validate()?;
                ensure!(
                    project(self.client()?.history(conversation)?)?
                        .iter()
                        .any(|m| m.message.message_id == reaction.target_message_id
                            && m.message.sender == reaction.target_sender),
                    "Reaction target unavailable"
                );
                let id: MessageId = serde_json::from_value(body["request_id"].clone())?;
                self.client()?.queue_message_once(
                    conversation,
                    id,
                    &serde_json::to_vec(&reaction)?,
                )?;
            }
            "typing" => {
                if self.online {
                    let event = Typing {
                        version: 1,
                        kind: TypingKind::Typing,
                        active: body["active"].as_bool().context("Invalid typing state")?,
                        expires_at: now() + TYPING_TTL,
                    };
                    self.client()?
                        .queue_transient(conversation, &serde_json::to_vec(&event)?)?;
                }
            }
            "read" => {
                if self.profile()?.read_receipts {
                    self.receipts(
                        conversation,
                        ReceiptState::Read,
                        Some(serde_json::from_value(body["targets"].clone())?),
                    )?;
                }
            }
            _ => bail!("Unsupported client route"),
        }
        // A network error does not discard or regenerate an accepted local send.
        let error = self.flush().await.err().map(|e| e.to_string());
        Ok(json!({"queued":true,"sync_error":error}))
    }
}
