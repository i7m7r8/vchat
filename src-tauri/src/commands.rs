use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::crypto;
use crate::crypto::store;
use crate::messaging;
use crate::webrtc;

// ═══════════════════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub public_key: String,
    pub onion_address: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub display_name: String,
    pub public_key: String,
    pub onion_address: String,
    pub added_at: i64,
    pub verified: bool,
    pub blocked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub content: String,
    pub timestamp: i64,
    pub encrypted: bool,
    pub message_type: MessageType,
    pub sequence_num: i64,
    pub reply_to: Option<String>,
    pub delivered: bool,
    pub read: bool,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Text,
    File,
    Image,
    Video,
    Audio,
    VoiceNote,
    Media,
    System,
    Sticker,
    Location,
}

impl MessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::File => "file",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::VoiceNote => "voice_note",
            Self::Media => "media",
            Self::System => "system",
            Self::Sticker => "sticker",
            Self::Location => "location",
        }
    }

    pub fn from_string(s: &str) -> Self {
        match s {
            "file" => Self::File,
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "voice_note" => Self::VoiceNote,
            "media" => Self::Media,
            "system" => Self::System,
            "sticker" => Self::Sticker,
            "location" => Self::Location,
            _ => Self::Text,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    pub id: String,
    pub message_id: String,
    pub sender: String,
    pub emoji: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypingStatus {
    pub peer_onion: String,
    pub is_typing: bool,
    pub last_typing_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_by: String,
    pub created_at: i64,
    pub member_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    pub group_id: String,
    pub onion_address: String,
    pub public_key: String,
    pub display_name: String,
    pub role: String,
    pub joined_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMessage {
    pub id: String,
    pub group_id: String,
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
    pub message_type: MessageType,
    pub reply_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallLogEntry {
    pub id: String,
    pub peer_onion: String,
    pub call_type: String,
    pub direction: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_secs: Option<i64>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransfer {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub status: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorStatus {
    pub connected: bool,
    pub onion_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionInfo {
    pub algorithm: String,
    pub key_exchange: String,
    pub key_derivation: String,
    pub signing: String,
    pub handshake: String,
    pub onion_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub disappearing_messages_default: bool,
    pub default_ttl_secs: u64,
    pub read_receipts: bool,
    pub typing_indicators: bool,
    pub notifications_enabled: bool,
    pub theme: String,
}

// Re-export CallSession from webrtc so the frontend can use it.
pub use webrtc::CallSession;

// ═══════════════════════════════════════════════════════════════════════════════
// Tauri-managed state
// ═══════════════════════════════════════════════════════════════════════════════

pub struct VchatState {
    pub webrtc: webrtc::SharedWebRTCState,
}

impl VchatState {
    pub fn new() -> Self {
        Self {
            webrtc: webrtc::create_state(),
        }
    }
}

impl Default for VchatState {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedVchatState = Arc<VchatState>;

pub fn create_app_state() -> SharedVchatState {
    Arc::new(VchatState::new())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════

async fn get_identity_onion() -> Result<String, String> {
    store::load_identity()
        .await
        .map_err(|e| e.to_string())?
        .map(|i| i.onion_address)
        .ok_or_else(|| "Identity not initialized".to_string())
}

fn parse_type_str(s: &str) -> MessageType {
    MessageType::from_string(s)}

fn now_ts() -> i64 {
    Utc::now().timestamp()
}

// ═══════════════════════════════════════════════════════════════════════════════
// Identity commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn init_identity(display_name: String) -> Result<Identity, String> {
    crypto::generate_identity(&display_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_identity() -> Result<Option<Identity>, String> {
    crypto::load_identity()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_onion_address() -> Result<String, String> {
    crate::tor::get_onion_address()
        .await
        .map_err(|e| e.to_string())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Contact commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn add_contact(
    display_name: String,
    public_key: String,
    onion_address: String,
) -> Result<Contact, String> {
    messaging::add_contact(&display_name, &public_key, &onion_address)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_contacts() -> Result<Vec<Contact>, String> {
    let contacts = store::load_contacts().await.map_err(|e| e.to_string())?;
    Ok(contacts)
}

#[tauri::command]
pub async fn delete_contact(onion_address: String) -> Result<(), String> {
    store::delete_contact(&onion_address)
        .await
        .map_err(|e| e.to_string())?;
    crate::error::audit_log("contact_deleted", &format!("onion={onion_address}"));
    Ok(())
}

#[tauri::command]
pub async fn block_contact(onion_address: String) -> Result<(), String> {
    store::block_contact(&onion_address, true)
        .await
        .map_err(|e| e.to_string())?;
    crate::error::audit_log("contact_blocked", &format!("onion={onion_address}"));
    Ok(())
}

#[tauri::command]
pub async fn unblock_contact(onion_address: String) -> Result<(), String> {
    store::block_contact(&onion_address, false)
        .await
        .map_err(|e| e.to_string())?;
    crate::error::audit_log("contact_unblocked", &format!("onion={onion_address}"));
    Ok(())
}

#[tauri::command]
pub async fn verify_contact(onion_address: String) -> Result<(), String> {
    store::verify_contact(&onion_address)
        .await
        .map_err(|e| e.to_string())?;
    crate::error::audit_log("contact_verified", &format!("onion={onion_address}"));
    Ok(())
}

#[tauri::command]
pub async fn get_contact(onion_address: String) -> Result<Option<Contact>, String> {
    let contact = store::load_single_contact(&onion_address)
        .await
        .map_err(|e| e.to_string())?;
    Ok(contact)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Message commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn send_message(
    recipient_onion: String,
    content: String,
    message_type: MessageType,
) -> Result<Message, String> {
    messaging::send_message(&recipient_onion, &content, message_type, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_reply_message(
    recipient_onion: String,
    content: String,
    message_type: MessageType,
    reply_to: String,
) -> Result<Message, String> {
    messaging::send_message(&recipient_onion, &content, message_type, Some(&reply_to))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_messages(contact_onion: String) -> Result<Vec<Message>, String> {
    let messages = store::load_messages(&contact_onion, 500, 0)
        .await
        .map_err(|e| e.to_string())?;

    Ok(messages.into_iter().rev().collect())
}

#[tauri::command]
pub async fn delete_message(message_id: String) -> Result<(), String> {
    store::delete_message(&message_id)
        .await
        .map_err(|e| e.to_string())?;
    crate::error::audit_log("message_deleted", &format!("id={message_id}"));
    Ok(())
}

#[tauri::command]
pub async fn search_messages(query: String) -> Result<Vec<Message>, String> {
    let messages = store::search_messages(&query, 100)
        .await
        .map_err(|e| e.to_string())?;
    Ok(messages)
}

#[tauri::command]
pub async fn mark_messages_read(contact_onion: String) -> Result<(), String> {
    let my_onion = get_identity_onion().await?;
    let db_messages = store::load_messages(&contact_onion, 10000, 0)
        .await
        .map_err(|e| e.to_string())?;

    for msg in db_messages {
        if msg.sender == contact_onion && msg.recipient == my_onion {
            store::mark_message_read(&msg.id)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn set_disappearing_message(message_id: String, ttl_secs: u64) -> Result<(), String> {
    store::set_message_ttl(&message_id, ttl_secs)
        .await
        .map_err(|e| e.to_string())?;
    crate::error::audit_log(
        "disappearing_message_set",
        &format!("id={message_id}, ttl={ttl_secs}s"),
    );
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Reaction commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn add_reaction(message_id: String, emoji: String) -> Result<Reaction, String> {
    let my_onion = get_identity_onion().await?;
    let id = uuid::Uuid::new_v4().to_string();

    store::save_reaction(&id, &message_id, &my_onion, &emoji)
        .await
        .map_err(|e| e.to_string())?;

    Ok(Reaction {
        id,
        message_id,
        sender: my_onion,
        emoji,
        timestamp: now_ts(),
    })
}

#[tauri::command]
pub async fn remove_reaction(message_id: String, emoji: String) -> Result<(), String> {
    let my_onion = get_identity_onion().await?;
    store::remove_reaction(&message_id, &my_onion, &emoji)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_reactions(message_id: String) -> Result<Vec<Reaction>, String> {
    let db_reactions = store::load_reactions(&message_id)
        .await
        .map_err(|e| e.to_string())?;

    let reactions = db_reactions
        .into_iter()
        .map(|(id, mid, sender, emoji, ts)| Reaction {
            id,
            message_id: mid,
            sender,
            emoji,
            timestamp: ts,
        })
        .collect();
    Ok(reactions)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Typing commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn send_typing_indicator(peer_onion: String, is_typing: bool) -> Result<(), String> {
    if is_typing {
        store::update_typing_indicator(&peer_onion)
            .await
            .map_err(|e| e.to_string())?;
    }
    crate::error::audit_log(
        "typing_indicator",
        &format!("peer={peer_onion}, typing={is_typing}"),
    );
    Ok(())
}

#[tauri::command]
pub async fn get_typing_status(peer_onion: String) -> Result<TypingStatus, String> {
    let last_typing_at = store::get_typing_indicator(&peer_onion)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or(0);

    let age = now_ts() - last_typing_at;
    let is_typing = last_typing_at > 0 && age < 5;

    Ok(TypingStatus {
        peer_onion,
        is_typing,
        last_typing_at,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// Group commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn create_group(name: String, description: String) -> Result<Group, String> {
    let my_onion = get_identity_onion().await?;
    let group_id = uuid::Uuid::new_v4().to_string();

    store::save_group(&group_id, &name, Some(&description), None, &my_onion)
        .await
        .map_err(|e| e.to_string())?;

    store::add_group_member(&group_id, &my_onion, None, None, "admin")
        .await
        .map_err(|e| e.to_string())?;

    crate::error::audit_log("group_created", &format!("id={group_id}, name={name}"));

    Ok(Group {
        id: group_id,
        name,
        description,
        created_by: my_onion,
        created_at: now_ts(),
        member_count: 1,
    })
}

#[tauri::command]
pub async fn get_groups() -> Result<Vec<Group>, String> {
    let db_groups = store::load_groups().await.map_err(|e| e.to_string())?;
    let mut groups = Vec::new();

    for (id, name, desc, _avatar, created_by, created_at, _updated_at) in db_groups {
        let members = store::load_group_members(&id)
            .await
            .map_err(|e| e.to_string())?;

        groups.push(Group {
            id,
            name,
            description: desc.unwrap_or_default(),
            created_by,
            created_at,
            member_count: members.len() as i64,
        });
    }

    Ok(groups)
}

#[tauri::command]
pub async fn get_group(group_id: String) -> Result<Option<Group>, String> {
    let db_groups = store::load_groups().await.map_err(|e| e.to_string())?;

    for (id, name, desc, _avatar, created_by, created_at, _updated_at) in db_groups {
        if id == group_id {
            let members = store::load_group_members(&id)
                .await
                .map_err(|e| e.to_string())?;

            return Ok(Some(Group {
                id,
                name,
                description: desc.unwrap_or_default(),
                created_by,
                created_at,
                member_count: members.len() as i64,
            }));
        }
    }

    Ok(None)
}

#[tauri::command]
pub async fn add_group_member(
    group_id: String,
    display_name: String,
    public_key: String,
    onion_address: String,
) -> Result<GroupMember, String> {
    store::add_group_member(&group_id, &onion_address, Some(&public_key), Some(&display_name), "member")
        .await
        .map_err(|e| e.to_string())?;

    crate::error::audit_log(
        "group_member_added",
        &format!("group={group_id}, onion={onion_address}"),
    );

    Ok(GroupMember {
        group_id,
        onion_address,
        public_key,
        display_name,
        role: "member".to_string(),
        joined_at: now_ts(),
    })
}

#[tauri::command]
pub async fn remove_group_member(group_id: String, onion_address: String) -> Result<(), String> {
    store::remove_group_member(&group_id, &onion_address)
        .await
        .map_err(|e| e.to_string())?;

    crate::error::audit_log(
        "group_member_removed",
        &format!("group={group_id}, onion={onion_address}"),
    );

    Ok(())
}

#[tauri::command]
pub async fn send_group_message(
    group_id: String,
    content: String,
    message_type: MessageType,
) -> Result<GroupMessage, String> {
    let my_onion = get_identity_onion().await?;
    let msg_id = uuid::Uuid::new_v4().to_string();
    let ts = now_ts();

    store::save_group_message(
        &msg_id,
        &group_id,
        &my_onion,
        Some(&content),
        None,
        ts,
        message_type.as_str(),
        None,
        None,
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::error::audit_log(
        "group_message_sent",
        &format!("group={group_id}, type={}", message_type.as_str()),
    );

    Ok(GroupMessage {
        id: msg_id,
        group_id,
        sender: my_onion,
        content,
        timestamp: ts,
        message_type,
        reply_to: None,
    })
}

#[tauri::command]
pub async fn get_group_messages(group_id: String) -> Result<Vec<GroupMessage>, String> {
    let db_messages = store::load_group_messages(&group_id, 500, 0)
        .await
        .map_err(|e| e.to_string())?;

    let messages = db_messages
        .into_iter()
        .rev()
        .map(|(id, gid, sender, content, _enc, ts, msg_type, _seq, reply_to)| {
            GroupMessage {
                id,
                group_id: gid,
                sender,
                content: content.unwrap_or_default(),
                timestamp: ts,
                message_type: parse_type_str(&msg_type),
                reply_to,
            }
        })
        .collect();

    Ok(messages)
}

#[tauri::command]
pub async fn get_group_members(group_id: String) -> Result<Vec<GroupMember>, String> {
    let db_members = store::load_group_members(&group_id)
        .await
        .map_err(|e| e.to_string())?;

    let members = db_members
        .into_iter()
        .map(|(gid, onion, pubkey, name, role, joined_at)| GroupMember {
            group_id: gid,
            onion_address: onion,
            public_key: pubkey.unwrap_or_default(),
            display_name: name.unwrap_or_default(),
            role,
            joined_at,
        })
        .collect();

    Ok(members)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Call commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn start_video_call(
    state: State<'_, SharedVchatState>,
    recipient_onion: String,
) -> Result<String, String> {
    let session = webrtc::start_video_call(state.webrtc.clone(), &recipient_onion)
        .await
        .map_err(|e| e.to_string())?;

    store::save_call_log(
        &session.call_id,
        &recipient_onion,
        "video",
        "outgoing",
        session.started_at.timestamp(),
        None,
        None,
        "ringing",
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::error::audit_log(
        "call_started",
        &format!("type=video, peer={recipient_onion}"),
    );

    Ok(session.call_id)
}

#[tauri::command]
pub async fn start_audio_call(
    state: State<'_, SharedVchatState>,
    recipient_onion: String,
) -> Result<String, String> {
    let session = webrtc::start_audio_call(state.webrtc.clone(), &recipient_onion)
        .await
        .map_err(|e| e.to_string())?;

    store::save_call_log(
        &session.call_id,
        &recipient_onion,
        "audio",
        "outgoing",
        session.started_at.timestamp(),
        None,
        None,
        "ringing",
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::error::audit_log(
        "call_started",
        &format!("type=audio, peer={recipient_onion}"),
    );

    Ok(session.call_id)
}

#[tauri::command]
pub async fn answer_video_call(
    state: State<'_, SharedVchatState>,
    call_id: String,
) -> Result<(), String> {
    webrtc::answer_video_call(state.webrtc.clone(), &call_id)
        .await
        .map_err(|e| e.to_string())?;

    crate::error::audit_log("call_answered", &format!("call_id={call_id}"));
    Ok(())
}

#[tauri::command]
pub async fn end_video_call(
    state: State<'_, SharedVchatState>,
    call_id: String,
) -> Result<(), String> {
    webrtc::end_video_call(state.webrtc.clone(), &call_id)
        .await
        .map_err(|e| e.to_string())?;

    crate::error::audit_log("call_ended", &format!("call_id={call_id}"));
    Ok(())
}

#[tauri::command]
pub async fn start_screen_share(
    state: State<'_, SharedVchatState>,
    call_id: String,
) -> Result<(), String> {
    webrtc::start_screen_share(state.webrtc.clone(), &call_id)
        .await
        .map_err(|e| e.to_string())?;

    crate::error::audit_log("screen_share_started", &format!("call_id={call_id}"));
    Ok(())
}

#[tauri::command]
pub async fn stop_screen_share(
    state: State<'_, SharedVchatState>,
    call_id: String,
) -> Result<(), String> {
    webrtc::stop_screen_share(state.webrtc.clone(), &call_id)
        .await
        .map_err(|e| e.to_string())?;

    crate::error::audit_log("screen_share_stopped", &format!("call_id={call_id}"));
    Ok(())
}

#[tauri::command]
pub async fn toggle_audio_mute(
    state: State<'_, SharedVchatState>,
    call_id: String,
) -> Result<bool, String> {
    webrtc::toggle_audio_mute(state.webrtc.clone(), &call_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_video(
    state: State<'_, SharedVchatState>,
    call_id: String,
) -> Result<bool, String> {
    webrtc::toggle_video(state.webrtc.clone(), &call_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_call_history() -> Result<Vec<CallLogEntry>, String> {
    let db_entries = store::load_call_log(200, 0)
        .await
        .map_err(|e| e.to_string())?;

    let entries = db_entries
        .into_iter()
        .map(|(id, peer, call_type, direction, started_at, ended_at, duration, status)| {
            CallLogEntry {
                id,
                peer_onion: peer,
                call_type,
                direction,
                started_at,
                ended_at,
                duration_secs: duration,
                status,
            }
        })
        .collect();

    Ok(entries)
}

#[tauri::command]
pub async fn get_active_calls(
    state: State<'_, SharedVchatState>,
) -> Result<Vec<CallSession>, String> {
    Ok(webrtc::get_active_calls(state.webrtc.clone()).await)
}

// ═══════════════════════════════════════════════════════════════════════════════
// File commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn send_file(
    recipient_onion: String,
    file_data: String,
    file_name: String,
    mime_type: String,
) -> Result<FileTransfer, String> {
    let my_onion = get_identity_onion().await?;

    let filename = if file_name.is_empty() {
        "unknown".to_string()
    } else {
        file_name
    };

    let mime = if mime_type.is_empty() {
        "application/octet-stream".to_string()
    } else {
        mime_type
    };

    let size = (file_data.len() * 3 / 4) as i64;

    let transfer_id = uuid::Uuid::new_v4().to_string();

    store::save_file_transfer(
        &transfer_id,
        &my_onion,
        &recipient_onion,
        &filename,
        Some(&mime),
        Some(size),
        None,
        None,
        "transferring",
    )
    .await
    .map_err(|e| e.to_string())?;

    crate::error::audit_log(
        "file_transfer_started",
        &format!("id={transfer_id}, to={recipient_onion}, file={filename}"),
    );

    Ok(FileTransfer {
        id: transfer_id,
        sender: my_onion,
        recipient: recipient_onion,
        filename,
        mime_type: mime,
        size,
        status: "transferring".to_string(),
        started_at: now_ts(),
        completed_at: None,
    })
}

#[tauri::command]
pub async fn get_file_transfers() -> Result<Vec<FileTransfer>, String> {
    let db_transfers = store::load_file_transfers(None, 200, 0)
        .await
        .map_err(|e| e.to_string())?;

    let transfers = db_transfers
        .into_iter()
        .map(|(id, sender, recipient, filename, mime, size, _key, _dir, status, started_at, completed_at)| {
            FileTransfer {
                id,
                sender,
                recipient,
                filename,
                mime_type: mime.unwrap_or_default(),
                size: size.unwrap_or(0),
                status,
                started_at,
                completed_at,
            }
        })
        .collect();

    Ok(transfers)
}

// ═══════════════════════════════════════════════════════════════════════════════
// QR commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn generate_qr_code() -> Result<String, String> {
    crypto::generate_qr_code()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn scan_qr_code(qr_data: String) -> Result<Contact, String> {
    let contact = crypto::parse_qr_data(&qr_data)
        .map_err(|e| e.to_string())?;

    let stored = messaging::add_contact(
        &contact.display_name,
        &contact.public_key,
        &contact.onion_address,
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(Contact {
        id: stored.id,
        display_name: stored.display_name,
        public_key: stored.public_key,
        onion_address: stored.onion_address,
        added_at: stored.added_at,
        verified: stored.verified,
        blocked: stored.blocked,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tor commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn get_tor_status() -> Result<TorStatus, String> {
    let connected = crate::tor::is_tor_ready().await;
    let onion_address = crate::tor::get_onion_address()
        .await
        .unwrap_or_default();

    Ok(TorStatus {
        connected,
        onion_address,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// Security commands
// ═══════════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn get_encryption_info() -> Result<EncryptionInfo, String> {
    Ok(EncryptionInfo {
        algorithm: "AES-256-GCM".to_string(),
        key_exchange: "X25519 ECDH".to_string(),
        key_derivation: "HKDF-SHA256".to_string(),
        signing: "Ed25519".to_string(),
        handshake: "Noise_XX_25519_ChaChaPoly_BLAKE2s".to_string(),
        onion_version: "v3 (ed25519)".to_string(),
    })
}

#[tauri::command]
pub async fn delete_all_data() -> Result<(), String> {
    store::delete_all_data()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Settings commands
// ═══════════════════════════════════════════════════════════════════════════════

fn default_settings() -> AppSettings {
    AppSettings {
        disappearing_messages_default: false,
        default_ttl_secs: 3600,
        read_receipts: true,
        typing_indicators: true,
        notifications_enabled: true,
        theme: "dark".to_string(),
    }
}

#[tauri::command]
pub async fn get_settings() -> Result<AppSettings, String> {
    let get = |key: String| async move {
        store::get_setting(&key)
            .await
            .map_err(|e| e.to_string())
    };

    let defaults = default_settings();

    let disappearing = get("disappearing_messages_default".to_string()).await?
        .map(|v| v == "true")
        .unwrap_or(defaults.disappearing_messages_default);

    let ttl = get("default_ttl_secs".to_string()).await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.default_ttl_secs);

    let read_receipts = get("read_receipts".to_string()).await?
        .map(|v| v == "true")
        .unwrap_or(defaults.read_receipts);

    let typing = get("typing_indicators".to_string()).await?
        .map(|v| v == "true")
        .unwrap_or(defaults.typing_indicators);

    let notifications = get("notifications_enabled".to_string()).await?
        .map(|v| v == "true")
        .unwrap_or(defaults.notifications_enabled);

    let theme = get("theme".to_string()).await?
        .unwrap_or(defaults.theme);

    Ok(AppSettings {
        disappearing_messages_default: disappearing,
        default_ttl_secs: ttl,
        read_receipts,
        typing_indicators: typing,
        notifications_enabled: notifications,
        theme,
    })
}

#[tauri::command]
pub async fn update_settings(settings: serde_json::Value) -> Result<AppSettings, String> {
    let mut current = get_settings().await?;

    if let Some(v) = settings.get("disappearing_messages_default").and_then(|v| v.as_bool()) {
        current.disappearing_messages_default = v;
    }
    if let Some(v) = settings.get("default_ttl_secs").and_then(|v| v.as_u64()) {
        current.default_ttl_secs = v;
    }
    if let Some(v) = settings.get("read_receipts").and_then(|v| v.as_bool()) {
        current.read_receipts = v;
    }
    if let Some(v) = settings.get("typing_indicators").and_then(|v| v.as_bool()) {
        current.typing_indicators = v;
    }
    if let Some(v) = settings.get("notifications_enabled").and_then(|v| v.as_bool()) {
        current.notifications_enabled = v;
    }
    if let Some(v) = settings.get("theme").and_then(|v| v.as_str()) {
        current.theme = v.to_string();
    }

    let persist = |key: &str, val: String| async move {
        store::set_setting(key, &val).await.map_err(|e| e.to_string())
    };

    persist("disappearing_messages_default", current.disappearing_messages_default.to_string()).await?;
    persist("default_ttl_secs", current.default_ttl_secs.to_string()).await?;
    persist("read_receipts", current.read_receipts.to_string()).await?;
    persist("typing_indicators", current.typing_indicators.to_string()).await?;
    persist("notifications_enabled", current.notifications_enabled.to_string()).await?;
    persist("theme", current.theme.clone()).await?;

    crate::error::audit_log("settings_updated", &serde_json::to_string(&settings).unwrap_or_default());

    Ok(current)
}
