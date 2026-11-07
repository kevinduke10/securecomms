//! Local-only multi-account test harness. This process hosts CLIENT encryption
//! engines as well as a ciphertext-only relay. Never deploy it as a cloud API.
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path as RoutePath, Query, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use croaker_client::ClientStore;
use croaker_crypto::{Device, DeviceBinding};
use croaker_protocol::*;
use croaker_relay::{Store, sqlite::SqliteStore};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;
use tower_http::services::ServeDir;
use zeroize::Zeroizing;

pub const MAX_ATTACHMENT: usize = 32 * 1024;
mod events;
use events::{ChatEvent, REACTION_EMOJIS, Reaction, ReactionKind, TYPING_TTL, Typing, TypingKind};

struct TypingState {
    sequence: u64,
    expires_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Profile {
    alias: String,
    name: String,
    title: String,
    principal: Principal,
    token: String,
    vault_key: String,
    binding: DeviceBinding,
}
#[derive(Clone, Serialize, Deserialize)]
struct Chat {
    id: ConversationId,
    title: String,
    kind: String,
    members: Vec<String>,
    owner: String,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u16,
    profiles: Vec<Profile>,
    chats: Vec<Chat>,
}

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
    fn validate(&self) -> Result<()> {
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

pub struct Workspace {
    directory: PathBuf,
    manifest: Manifest,
    clients: HashMap<String, ClientStore>,
    online: HashMap<String, bool>,
    relay: SqliteStore,
    typing: HashMap<(ConversationId, DeviceId), TypingState>,
    outgoing_typing: HashMap<(ConversationId, DeviceId), u64>,
}

fn random_key() -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0; 32]);
    OsRng.fill_bytes(key.as_mut());
    key
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temp = path.with_extension(format!("{}.tmp", MessageId::new()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

impl Workspace {
    pub async fn open(directory: PathBuf) -> Result<Self> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        let relay = SqliteStore::open(directory.join("relay.sqlite"))?;
        let path = directory.join("workspace.local.json");
        let mut clients = HashMap::new();
        let fresh = !path.exists();
        let manifest = if fresh {
            // Do not overwrite cryptographic state from a partially initialized
            // workspace. A fresh directory is required for a new fixture.
            ensure!(
                !directory.join("alice.sqlite").exists(),
                "Incomplete local workspace. Start with a fresh --data-dir; existing client data was preserved."
            );
            let tenant = TenantId::new();
            let mut profiles = Vec::new();
            for (alias, name, title, role) in [
                ("alice", "Alice Morgan", "Product & strategy", Role::Admin),
                ("bob", "Bob Chen", "Engineering", Role::Member),
                ("maya", "Maya Patel", "Design partner", Role::Guest),
            ] {
                let principal = Principal {
                    tenant_id: tenant,
                    device_id: DeviceId::new(),
                    role,
                };
                let device = Device::new(tenant, principal.device_id)?;
                let binding = device.binding().clone();
                let key = random_key();
                let profile = Profile {
                    alias: alias.into(),
                    name: name.into(),
                    title: title.into(),
                    principal,
                    token: hex::encode(*random_key()),
                    vault_key: hex::encode(*key),
                    binding,
                };
                relay.provision(&profile.principal, &profile.token).await?;
                clients.insert(
                    alias.into(),
                    ClientStore::create(directory.join(format!("{alias}.sqlite")), device, key)?,
                );
                profiles.push(profile);
            }
            Manifest {
                version: 1,
                profiles,
                chats: Vec::new(),
            }
        } else {
            let bytes = Zeroizing::new(std::fs::read(path)?);
            let manifest: Manifest = serde_json::from_slice(&bytes)?;
            ensure!(
                manifest.version == 1,
                "Unsupported local workspace version."
            );
            for profile in &manifest.profiles {
                let bytes = Zeroizing::new(hex::decode(&profile.vault_key)?);
                let key: [u8; 32] = bytes
                    .as_slice()
                    .try_into()
                    .context("Invalid local vault key")?;
                clients.insert(
                    profile.alias.clone(),
                    ClientStore::open(
                        directory.join(format!("{}.sqlite", profile.alias)),
                        Zeroizing::new(key),
                    )?,
                );
            }
            manifest
        };
        let online = manifest
            .profiles
            .iter()
            .map(|p| (p.alias.clone(), true))
            .collect();
        let mut workspace = Self {
            directory,
            manifest,
            clients,
            online,
            relay,
            typing: HashMap::new(),
            outgoing_typing: HashMap::new(),
        };
        if fresh {
            workspace.save()?;
            let team = workspace
                .create_chat(
                    "alice",
                    "Launch team".into(),
                    "group".into(),
                    vec!["bob".into(), "maya".into()],
                )
                .await?;
            let notes = workspace
                .create_chat(
                    "alice",
                    "Product notes".into(),
                    "group".into(),
                    vec!["bob".into()],
                )
                .await?;
            let direct = workspace
                .create_chat("alice", "".into(), "direct".into(), vec!["bob".into()])
                .await?;
            workspace
                .create_chat("alice", "".into(), "direct".into(), vec!["maya".into()])
                .await?;
            for (profile, chat, text) in [
                (
                    "alice",
                    team,
                    "A quieter place for our next big idea. Welcome to the launch team 👋",
                ),
                (
                    "bob",
                    team,
                    "Already feels like home. I’ve got the rollout checklist ready.",
                ),
                (
                    "maya",
                    team,
                    "And I’m wrapping up the launch visuals. I’ll share a first look here.",
                ),
                (
                    "alice",
                    team,
                    "Perfect. Let’s use this space for decisions, feedback, and the occasional good idea.",
                ),
                ("bob", team, "Shall we aim for a first review on Thursday?"),
                ("maya", team, "Thursday works for me ✨"),
                (
                    "alice",
                    team,
                    "Thursday it is. Looking forward to seeing it come together.",
                ),
                (
                    "bob",
                    notes,
                    "A place for the little things that turn into great features.",
                ),
                (
                    "alice",
                    direct,
                    "Hey Bob! Ready to put our new workspace through its paces?",
                ),
                ("bob", direct, "Absolutely. Send something my way."),
            ] {
                workspace
                    .send(
                        profile,
                        chat,
                        MessageId::new(),
                        Content {
                            version: 1,
                            text: text.into(),
                            attachment: None,
                        },
                    )
                    .await?;
                workspace.sync_all().await?;
            }
        }
        Ok(workspace)
    }

    fn save(&self) -> Result<()> {
        let bytes = Zeroizing::new(serde_json::to_vec_pretty(&self.manifest)?);
        write_private(&self.directory.join("workspace.local.json"), &bytes)
    }
    fn profile(&self, alias: &str) -> Result<Profile> {
        self.manifest
            .profiles
            .iter()
            .find(|p| p.alias == alias)
            .cloned()
            .context("Unknown local test account.")
    }
    fn member_chat(&self, alias: &str, id: ConversationId) -> Result<Chat> {
        self.manifest
            .chats
            .iter()
            .find(|c| c.id == id && c.members.iter().any(|m| m == alias))
            .cloned()
            .context("Conversation not available to this account.")
    }
    async fn flush(&mut self, alias: &str) -> Result<()> {
        if !self.online.get(alias).copied().unwrap_or(false) {
            return Ok(());
        }
        let profile = self.profile(alias)?;
        loop {
            let pending = self.clients[alias].pending()?;
            if pending.is_empty() {
                break;
            }
            let actor = self.relay.authenticate(&profile.token).await?;
            for pending in pending {
                self.relay
                    .append(&actor, pending.conversation_id, pending.event.clone())
                    .await?;
                self.clients
                    .get_mut(alias)
                    .unwrap()
                    .acknowledge(pending.event.message_id)?;
            }
        }
        Ok(())
    }
    async fn sync_profile(&mut self, alias: &str) -> Result<()> {
        let profile = self.profile(alias)?;
        if !self.online.get(alias).copied().unwrap_or(false) {
            return Ok(());
        }
        self.flush(alias).await?;
        let chats: Vec<_> = self
            .manifest
            .chats
            .iter()
            .filter(|c| c.members.iter().any(|m| m == alias))
            .cloned()
            .collect();
        let actor = self.relay.authenticate(&profile.token).await?;
        for chat in chats {
            let peers: Vec<_> = chat
                .members
                .iter()
                .map(|a| self.profile(a).map(|p| p.binding))
                .collect::<Result<_>>()?;
            loop {
                let cursor = self.clients[alias].cursor(chat.id)?;
                let page = self
                    .relay
                    .sync(&actor, chat.id, cursor, MAX_SYNC_PAGE)
                    .await?;
                let transient = self
                    .clients
                    .get_mut(alias)
                    .unwrap()
                    .apply_page_with_transient(
                        chat.id,
                        cursor,
                        &page,
                        &peers,
                        events::is_transient,
                    )?;
                for signal in transient {
                    if let Ok(ChatEvent::Typing(event)) =
                        serde_json::from_slice(&signal.message.plaintext)
                    {
                        let now = croaker_relay::now();
                        if event.version != 1 {
                            continue;
                        }
                        let expires_at = if event.active && event.expires_at <= now + TYPING_TTL + 2
                        {
                            event.expires_at
                        } else {
                            0
                        };
                        let entry = self
                            .typing
                            .entry((chat.id, signal.message.sender))
                            .or_insert(TypingState {
                                sequence: 0,
                                expires_at: 0,
                            });
                        if signal.sequence > entry.sequence {
                            *entry = TypingState {
                                sequence: signal.sequence,
                                expires_at,
                            };
                        }
                    }
                }
                if !page.has_more {
                    break;
                }
            }
        }
        Ok(())
    }
    async fn sync_all(&mut self) -> Result<()> {
        let profiles: Vec<_> = self
            .manifest
            .profiles
            .iter()
            .map(|p| p.alias.clone())
            .collect();
        for alias in profiles {
            self.sync_profile(&alias).await?;
        }
        Ok(())
    }

    pub async fn create_chat(
        &mut self,
        alias: &str,
        title: String,
        kind: String,
        mut members: Vec<String>,
    ) -> Result<ConversationId> {
        let profile = self.profile(alias)?;
        ensure!(
            profile.principal.role != Role::Guest,
            "Guest accounts can participate in existing conversations, but cannot create new ones."
        );
        ensure!(
            self.online[alias],
            "Reconnect before starting a conversation."
        );
        ensure!(
            kind == "group" || kind == "direct",
            "Choose a group or direct conversation."
        );
        let title = title.trim().to_string();
        ensure!(
            kind == "direct" || (2..=48).contains(&title.chars().count()),
            "Group names must contain 2–48 characters."
        );
        members.push(alias.into());
        members.sort();
        members.dedup();
        ensure!(
            (2..=3).contains(&members.len()) && (kind != "direct" || members.len() == 2),
            "Choose at least one person; direct chats have two members."
        );
        for member in &members {
            self.profile(member)?;
        }
        if kind == "direct"
            && let Some(existing) = self
                .manifest
                .chats
                .iter()
                .find(|c| c.kind == "direct" && c.members == members)
        {
            return Ok(existing.id);
        }
        let id = ConversationId::new();
        let mut packages = Vec::new();
        for member in members.iter().filter(|m| m.as_str() != alias) {
            let binding = self.profile(member)?.binding;
            let package = self
                .clients
                .get_mut(member)
                .unwrap()
                .update(Device::key_package)?;
            packages.push((package, binding));
        }
        let actor = self.relay.authenticate(&profile.token).await?;
        let devices = members
            .iter()
            .map(|m| self.profile(m).map(|p| p.principal.device_id))
            .collect::<Result<_>>()?;
        self.relay
            .create(&actor, CreateConversation { id, devices })
            .await?;
        self.clients
            .get_mut(alias)
            .unwrap()
            .create_group(id, &packages)?;
        self.manifest.chats.push(Chat {
            id,
            title,
            kind,
            members,
            owner: alias.into(),
        });
        self.save()?;
        self.flush(alias).await?;
        Ok(id)
    }

    pub async fn send(
        &mut self,
        alias: &str,
        conversation: ConversationId,
        request_id: MessageId,
        content: Content,
    ) -> Result<()> {
        content.validate()?;
        self.member_chat(alias, conversation)?;
        self.sync_profile(alias).await?;
        let bytes = Zeroizing::new(serde_json::to_vec(&content)?);
        self.clients
            .get_mut(alias)
            .context("Unknown account")?
            .queue_message_once(conversation, request_id, &bytes)?;
        self.flush(alias).await?;
        // Typing is optional; an already accepted message must not fail merely
        // because a best-effort stop signal could not be delivered.
        let _ = self.publish_typing(alias, conversation, false).await;
        Ok(())
    }

    pub async fn react(
        &mut self,
        alias: &str,
        conversation: ConversationId,
        request_id: MessageId,
        target_message_id: MessageId,
        target_alias: &str,
        emoji: Option<String>,
    ) -> Result<()> {
        self.member_chat(alias, conversation)?;
        let target = self.profile(target_alias)?;
        self.sync_profile(alias).await?;
        let reaction = Reaction {
            version: 1,
            kind: ReactionKind::Reaction,
            target_message_id,
            target_sender: target.principal.device_id,
            emoji,
        };
        reaction.validate()?;
        let exists = events::project(self.clients[alias].history(conversation)?)?
            .iter()
            .any(|m| {
                m.message.message_id == target_message_id
                    && m.message.sender == target.principal.device_id
            });
        ensure!(
            exists,
            "That message is not available in this conversation."
        );
        let bytes = Zeroizing::new(serde_json::to_vec(&reaction)?);
        self.clients
            .get_mut(alias)
            .context("Unknown account")?
            .queue_message_once(conversation, request_id, &bytes)?;
        self.flush(alias).await?;
        Ok(())
    }

    pub async fn publish_typing(
        &mut self,
        alias: &str,
        conversation: ConversationId,
        active: bool,
    ) -> Result<()> {
        self.member_chat(alias, conversation)?;
        let profile = self.profile(alias)?;
        let key = (conversation, profile.principal.device_id);
        if !self.online[alias] {
            self.outgoing_typing.remove(&key);
            return Ok(());
        }
        let now = croaker_relay::now();
        let previous = self.outgoing_typing.get(&key).copied().unwrap_or(0);
        // At most one active pulse every two seconds; repeated stops are no-ops.
        if (active && previous > now + TYPING_TTL - 2) || (!active && previous == 0) {
            return Ok(());
        }
        self.sync_profile(alias).await?;
        let event = Typing {
            version: 1,
            kind: TypingKind::Typing,
            active,
            expires_at: if active { now + TYPING_TTL } else { 0 },
        };
        let bytes = Zeroizing::new(serde_json::to_vec(&event)?);
        self.clients
            .get_mut(alias)
            .unwrap()
            .queue_transient(conversation, &bytes)?;
        self.outgoing_typing.insert(key, event.expires_at);
        self.flush(alias).await?;
        Ok(())
    }

    pub async fn snapshot(&mut self, alias: &str) -> Result<serde_json::Value> {
        self.sync_profile(alias).await?;
        let current = self.profile(alias)?;
        let profiles: Vec<_> = self.manifest.profiles.iter().map(|p| serde_json::json!({"id":p.alias,"name":p.name,"title":p.title,"role":p.principal.role,"online":self.online[&p.alias],"fingerprint":p.binding.fingerprint()})).collect();
        let mut chats = Vec::new();
        for chat in &self.manifest.chats {
            if !chat.members.iter().any(|m| m == alias) {
                continue;
            }
            let mut messages = Vec::new();
            for projected in events::project(self.clients[alias].history(chat.id)?)? {
                let message = projected.message;
                let content = projected.content;
                let sender = self
                    .manifest
                    .profiles
                    .iter()
                    .find(|p| p.principal.device_id == message.sender)
                    .context("Unknown message sender")?;
                let mut grouped = std::collections::BTreeMap::<String, (Vec<String>, bool)>::new();
                for (device, (emoji, queued)) in &projected.reactions {
                    if let Some(profile) = self
                        .manifest
                        .profiles
                        .iter()
                        .find(|p| p.principal.device_id == *device)
                    {
                        let entry = grouped.entry(emoji.clone()).or_default();
                        entry.0.push(profile.alias.clone());
                        entry.1 |= *queued;
                    }
                }
                let mut grouped: Vec<_> = grouped.into_iter().collect();
                grouped.sort_by_key(|(emoji, _)| {
                    REACTION_EMOJIS
                        .iter()
                        .position(|value| value == emoji)
                        .unwrap_or(usize::MAX)
                });
                let reactions: Vec<_> = grouped.into_iter().map(|(emoji,(users,pending))|serde_json::json!({"emoji":emoji,"users":users,"pending":pending})).collect();
                messages.push(serde_json::json!({"id":message.message_id,"sender":sender.alias,"sent_at":message.sent_at,"text":content.text,"attachment":content.attachment,"pending":message.pending,"reactions":reactions}));
            }
            let title = if chat.kind == "direct" {
                self.profile(chat.members.iter().find(|m| m.as_str() != alias).unwrap())?
                    .name
            } else {
                chat.title.clone()
            };
            let typing: Vec<_> = self
                .manifest
                .profiles
                .iter()
                .filter(|p| {
                    p.alias != alias
                        && self.online[alias]
                        && self.online[&p.alias]
                        && chat.members.contains(&p.alias)
                        && self
                            .typing
                            .get(&(chat.id, p.principal.device_id))
                            .is_some_and(|s| s.expires_at > croaker_relay::now())
                })
                .map(|p| p.alias.clone())
                .collect();
            chats.push(serde_json::json!({"id":chat.id,"title":title,"kind":chat.kind,"members":chat.members,"owner":chat.owner,"messages":messages,"typing":typing}));
        }
        Ok(
            serde_json::json!({"test_mode":true,"current_profile":current.alias,"profiles":profiles,"conversations":chats,"online":self.online[alias],"attachment_limit":MAX_ATTACHMENT,"reaction_options":REACTION_EMOJIS}),
        )
    }
}

type Shared = Arc<Mutex<Workspace>>;
#[derive(Deserialize)]
struct AccountQuery {
    profile: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendInput {
    conversation_id: ConversationId,
    request_id: MessageId,
    content: Content,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReactionInput {
    conversation_id: ConversationId,
    request_id: MessageId,
    target_message_id: MessageId,
    target_sender: String,
    emoji: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypingInput {
    conversation_id: ConversationId,
    active: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    title: String,
    kind: String,
    members: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OnlineInput {
    online: bool,
}
struct LocalError(anyhow::Error);
impl From<anyhow::Error> for LocalError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for LocalError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":self.0.to_string()})),
        )
            .into_response()
    }
}
async fn snapshot(
    State(workspace): State<Shared>,
    Query(query): Query<AccountQuery>,
) -> std::result::Result<Json<serde_json::Value>, LocalError> {
    Ok(Json(workspace.lock().await.snapshot(&query.profile).await?))
}
async fn send(
    State(workspace): State<Shared>,
    RoutePath(alias): RoutePath<String>,
    Json(input): Json<SendInput>,
) -> std::result::Result<StatusCode, LocalError> {
    workspace
        .lock()
        .await
        .send(
            &alias,
            input.conversation_id,
            input.request_id,
            input.content,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn create(
    State(workspace): State<Shared>,
    RoutePath(alias): RoutePath<String>,
    Json(input): Json<CreateInput>,
) -> std::result::Result<Json<serde_json::Value>, LocalError> {
    let id = workspace
        .lock()
        .await
        .create_chat(&alias, input.title, input.kind, input.members)
        .await?;
    Ok(Json(serde_json::json!({"id":id})))
}
async fn react(
    State(workspace): State<Shared>,
    RoutePath(alias): RoutePath<String>,
    Json(input): Json<ReactionInput>,
) -> std::result::Result<StatusCode, LocalError> {
    workspace
        .lock()
        .await
        .react(
            &alias,
            input.conversation_id,
            input.request_id,
            input.target_message_id,
            &input.target_sender,
            input.emoji,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn typing(
    State(workspace): State<Shared>,
    RoutePath(alias): RoutePath<String>,
    Json(input): Json<TypingInput>,
) -> std::result::Result<StatusCode, LocalError> {
    workspace
        .lock()
        .await
        .publish_typing(&alias, input.conversation_id, input.active)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn online(
    State(workspace): State<Shared>,
    RoutePath(alias): RoutePath<String>,
    Json(input): Json<OnlineInput>,
) -> std::result::Result<StatusCode, LocalError> {
    let mut workspace = workspace.lock().await;
    let profile = workspace.profile(&alias)?;
    if !input.online {
        let active: Vec<_> = workspace
            .outgoing_typing
            .iter()
            .filter(|((_, device), expires)| {
                *device == profile.principal.device_id && **expires > 0
            })
            .map(|((chat, _), _)| *chat)
            .collect();
        for chat in active {
            workspace.publish_typing(&alias, chat, false).await?;
        }
    }
    workspace.online.insert(alias.clone(), input.online);
    workspace.sync_profile(&alias).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn router(workspace: Workspace, assets: PathBuf, port: u16) -> Router {
    let hosts: Vec<String> = ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .map(|host| format!("{host}:{port}"))
        .collect();
    let api = Router::new().route("/state",get(snapshot)).route("/{profile}/send",post(send)).route("/{profile}/conversations",post(create)).route("/{profile}/online",post(online)).route("/{profile}/reactions",post(react)).route("/{profile}/typing",post(typing))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn(move |request: axum::extract::Request,next: Next| {
            let hosts = hosts.clone();
            async move {
                let headers = request.headers();
                let host_ok = headers.get("host").and_then(|h|h.to_str().ok()).is_some_and(|host|hosts.iter().any(|h|h == host));
                let origin_ok = headers.get("origin").is_none_or(|h|h.to_str().ok().is_some_and(|origin|hosts.iter().any(|h|origin == format!("http://{h}"))));
                let site_ok = headers.get("sec-fetch-site").is_none_or(|h|matches!(h.to_str(),Ok("same-origin" | "none")));
                if !host_ok || !origin_ok || !site_ok || headers.get("x-croaker-local").and_then(|h|h.to_str().ok()) != Some("1") {
                    return (StatusCode::FORBIDDEN,Json(serde_json::json!({"error":"Local workspace requests must come from this app."}))).into_response();
                }
                next.run(request).await
            }
        })).with_state(Arc::new(Mutex::new(workspace)));
    Router::new().nest("/local-api",api).route("/healthz",get(||async {"ok"})).fallback_service(ServeDir::new(assets))
        .layer(middleware::map_response(|mut response: Response|async move {
            let headers = response.headers_mut();
            headers.insert("cache-control",HeaderValue::from_static("no-store"));
            headers.insert("x-content-type-options",HeaderValue::from_static("nosniff"));
            headers.insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none'; object-src 'none'; base-uri 'none'; form-action 'self'"));
            response
        }))
}
