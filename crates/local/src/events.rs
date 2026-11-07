use crate::Content;
use anyhow::{Result, ensure};
use croaker_client::HistoryMessage;
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

#[derive(Deserialize)]
#[serde(untagged)]
pub enum ChatEvent {
    Message(Content),
    Reaction(Reaction),
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
    /// One reaction per authenticated sender, with local pending status.
    pub reactions: BTreeMap<DeviceId, (String, bool)>,
}

pub fn project(history: Vec<HistoryMessage>) -> Result<Vec<ProjectedMessage>> {
    let mut messages = Vec::new();
    let mut indexes = HashMap::new();
    let mut reactions = Vec::new();
    for message in history {
        match serde_json::from_slice::<ChatEvent>(&message.plaintext)? {
            ChatEvent::Message(content) => {
                indexes.insert((message.sender, message.message_id), messages.len());
                messages.push(ProjectedMessage {
                    message,
                    content,
                    reactions: BTreeMap::new(),
                });
            }
            ChatEvent::Reaction(reaction) => {
                // Invalid or orphaned control events never become chat messages.
                if reaction.validate().is_ok() {
                    reactions.push((message.sender, message.pending, reaction));
                }
            }
            ChatEvent::Typing(_) => {} // Also tolerate history from older development builds.
        }
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
    Ok(messages)
}
