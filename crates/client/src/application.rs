//! Shared authenticated application payloads and history projection.
use crate::HistoryMessage;
use anyhow::{Context, Result, ensure};
use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_protocol::{DeviceId, MessageId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::OnceLock,
};

pub const REACTION_EMOJIS: &[&str] = &["👍", "❤️", "😂", "🎉", "😮", "😢"];
pub const TYPING_TTL: u64 = 6;

fn supported_emoji(emoji: &str) -> bool {
    #[derive(Deserialize)]
    struct Entry {
        emoji: String,
    }
    static CATALOG: OnceLock<HashSet<String>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let entries: Vec<Entry> =
                serde_json::from_str(include_str!("../../../shared/emoji.json"))
                    .expect("bundled emoji catalog is valid");
            entries.into_iter().map(|entry| entry.emoji).collect()
        })
        .contains(emoji)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReactionKind {
    Reaction,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reaction {
    pub version: u16,
    pub kind: ReactionKind,
    pub target_message_id: MessageId,
    pub target_sender: DeviceId,
    /// Absolute per-person state: a new emoji replaces the previous one; null removes it.
    pub emoji: Option<String>,
}

impl Reaction {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.emoji.as_deref().is_none_or(supported_emoji),
            "Choose one of the supported reaction emojis."
        );
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypingKind {
    Typing,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Typing {
    pub version: u16,
    pub kind: TypingKind,
    pub active: bool,
    pub expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageRef {
    pub message_id: MessageId,
    pub sender: DeviceId,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyKind {
    Reply,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub version: u16,
    pub kind: ReplyKind,
    pub reply_to: MessageRef,
    pub content: Content,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptKind {
    Receipt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptState {
    Delivered,
    Read,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u16,
    pub kind: ReceiptKind,
    pub state: ReceiptState,
    pub targets: Vec<MessageRef>,
}
impl Receipt {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && !self.targets.is_empty() && self.targets.len() <= 64,
            "Invalid receipt event."
        );
        let unique: HashSet<_> = self
            .targets
            .iter()
            .map(|r| (r.sender, r.message_id))
            .collect();
        ensure!(
            unique.len() == self.targets.len(),
            "Duplicate receipt targets."
        );
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Media,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaEvent {
    pub version: u16,
    pub kind: MediaKind,
    pub text: String,
    pub attachment: croaker_protocol::media::ObjectAttachment,
    pub reply_to: Option<MessageRef>,
}
impl MediaEvent {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.text.len() <= 8000,
            "Invalid media message."
        );
        self.attachment.validate().map_err(anyhow::Error::msg)?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum ChatEvent {
    Media(MediaEvent),
    Message(Content),
    Reaction(Reaction),
    Reply(Reply),
    Receipt(Receipt),
    Typing(Typing),
}

pub fn is_transient(bytes: &[u8]) -> bool {
    matches!(
        serde_json::from_slice::<ChatEvent>(bytes),
        Ok(ChatEvent::Typing(_))
    )
}

pub struct ProjectedMessage {
    pub message: HistoryMessage,
    pub content: Content,
    pub media: Option<croaker_protocol::media::ObjectAttachment>,
    /// One reaction per authenticated sender, with local pending status.
    pub reactions: BTreeMap<DeviceId, (String, bool)>,
    /// References contain no quoted user-supplied text; snapshot resolves the target.
    pub reply_to: Option<MessageRef>,
    /// Monotonic authenticated recipient claims, per device; pending local events
    /// are included to deduplicate retries, never to claim another device read.
    pub receipts: BTreeMap<DeviceId, ReceiptState>,
}

pub fn project(history: Vec<HistoryMessage>) -> Result<Vec<ProjectedMessage>> {
    let mut messages = Vec::new();
    let mut indexes = HashMap::new();
    let mut reactions = Vec::new();
    let mut receipts = Vec::new();
    for message in history {
        let mut media = None;
        let (content, reply_to) = match serde_json::from_slice::<ChatEvent>(&message.plaintext)? {
            ChatEvent::Message(content) => (content, None),
            ChatEvent::Media(event) => {
                if event.validate().is_err()
                    || event.attachment.conversation_id != message.conversation_id
                {
                    continue;
                }
                media = Some(event.attachment);
                (
                    Content {
                        version: 1,
                        text: event.text,
                        attachment: None,
                    },
                    event.reply_to,
                )
            }
            ChatEvent::Reply(reply) => {
                if reply.version != 1 || reply.content.validate().is_err() {
                    continue;
                }
                (reply.content, Some(reply.reply_to))
            }
            ChatEvent::Reaction(reaction) => {
                if reaction.validate().is_ok() {
                    reactions.push((message.sender, message.pending, reaction));
                }
                continue;
            }
            ChatEvent::Receipt(receipt) => {
                if receipt.validate().is_ok() {
                    receipts.push((message.sender, receipt));
                }
                continue;
            }
            ChatEvent::Typing(_) => continue,
        };
        indexes.insert((message.sender, message.message_id), messages.len());
        messages.push(ProjectedMessage {
            message,
            content,
            media,
            reactions: BTreeMap::new(),
            reply_to,
            receipts: BTreeMap::new(),
        });
    }
    for (actor, pending, reaction) in reactions {
        if let Some(&index) = indexes.get(&(reaction.target_sender, reaction.target_message_id)) {
            if let Some(emoji) = reaction.emoji {
                messages[index].reactions.insert(actor, (emoji, pending));
            } else {
                messages[index].reactions.remove(&actor);
            }
        }
    }
    for (actor, receipt) in receipts {
        for target in receipt.targets {
            if actor == target.sender {
                continue;
            }
            if let Some(&index) = indexes.get(&(target.sender, target.message_id)) {
                let previous = messages[index]
                    .receipts
                    .entry(actor)
                    .or_insert(receipt.state);
                *previous = (*previous).max(receipt.state);
            }
        }
    }
    Ok(messages)
}

pub const MAX_ATTACHMENT: usize = 32 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data: String,
    pub size: usize,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub version: u16,
    pub text: String,
    pub attachment: Option<Attachment>,
}

impl Content {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.text.len() <= 8_000,
            "Messages can contain up to 8,000 bytes of text."
        );
        ensure!(
            !self.text.trim().is_empty() || self.attachment.is_some(),
            "Write a message or choose a file."
        );
        if let Some(file) = &self.attachment {
            ensure!(
                !file.name.is_empty()
                    && file.name.len() <= 180
                    && !file
                        .name
                        .chars()
                        .any(|c| c.is_control() || c == '/' || c == '\\'),
                "Choose a file with a valid name."
            );
            ensure!(
                file.mime.len() <= 100 && file.data.len() <= MAX_ATTACHMENT.div_ceil(3) * 4,
                "Local demo attachments must be 32 KB or smaller."
            );
            let bytes = BASE64_STANDARD
                .decode(&file.data)
                .context("Invalid attachment encoding.")?;
            ensure!(
                bytes.len() == file.size && bytes.len() <= MAX_ATTACHMENT,
                "Local demo attachments must be 32 KB or smaller."
            );
        }
        Ok(())
    }
}
