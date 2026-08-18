use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CallType {
    Audio,
    Video,
    ScreenShare,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CallStatus {
    Initiating,
    Ringing,
    Connected,
    Muted,
    ScreenSharing,
    Ended,
    Missed,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallSession {
    pub call_id: String,
    pub peer_onion: String,
    pub is_initiator: bool,
    pub status: CallStatus,
    pub started_at: DateTime<Utc>,
    pub call_type: CallType,
    pub audio_muted: bool,
    pub video_enabled: bool,
    pub screen_sharing: bool,
    pub ice_candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallLogEntry {
    pub id: String,
    pub peer_onion: String,
    pub call_type: CallType,
    pub direction: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<i64>,
    pub status: CallStatus,
}

pub struct WebRTCState {
    pub active_calls: HashMap<String, CallSession>,
    pub call_history: Vec<CallLogEntry>,
}

impl WebRTCState {
    pub fn new() -> Self {
        Self {
            active_calls: HashMap::new(),
            call_history: Vec::new(),
        }
    }
}

pub type SharedWebRTCState = Arc<RwLock<WebRTCState>>;

pub fn create_state() -> SharedWebRTCState {
    Arc::new(RwLock::new(WebRTCState::new()))
}

fn update_call_status(call: &mut CallSession, status: CallStatus) {
    debug!(call_id = %call.call_id, ?status, "updating call status");
    call.status = status;
}

fn build_session(
    call_id: &str,
    peer_onion: &str,
    is_initiator: bool,
    call_type: CallType,
) -> CallSession {
    CallSession {
        call_id: call_id.to_string(),
        peer_onion: peer_onion.to_string(),
        is_initiator,
        status: CallStatus::Initiating,
        started_at: Utc::now(),
        call_type,
        audio_muted: false,
        video_enabled: true,
        screen_sharing: false,
        ice_candidates: Vec::new(),
    }
}

async fn establish_call_connection(state: SharedWebRTCState, call_id: &str) -> Result<()> {
    let mut s = state.write().await;
    let call = s
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;

    update_call_status(call, CallStatus::Connected);
    info!(call_id = %call_id, peer = %call.peer_onion, "call connected");

    let entry = CallLogEntry {
        id: Uuid::new_v4().to_string(),
        peer_onion: call.peer_onion.clone(),
        call_type: call.call_type.clone(),
        direction: if call.is_initiator { "outgoing" } else { "incoming" }.to_string(),
        started_at: call.started_at,
        ended_at: None,
        duration_secs: None,
        status: CallStatus::Connected,
    };
    s.call_history.push(entry);

    Ok(())
}

pub async fn start_video_call(state: SharedWebRTCState, recipient_onion: &str) -> Result<CallSession> {
    let call_id = Uuid::new_v4().to_string();
    info!(call_id = %call_id, peer = %recipient_onion, "starting video call");

    let session = build_session(&call_id, recipient_onion, true, CallType::Video);

    let mut s = state.write().await;
    s.active_calls.insert(call_id.clone(), session.clone());

    establish_call_connection(state.clone(), &call_id).await?;
    Ok(session)
}

pub async fn start_audio_call(state: SharedWebRTCState, recipient_onion: &str) -> Result<CallSession> {
    let call_id = Uuid::new_v4().to_string();
    info!(call_id = %call_id, peer = %recipient_onion, "starting audio call");

    let mut session = build_session(&call_id, recipient_onion, true, CallType::Audio);
    session.video_enabled = false;

    let mut s = state.write().await;
    s.active_calls.insert(call_id.clone(), session.clone());

    establish_call_connection(state.clone(), &call_id).await?;
    Ok(session)
}

pub async fn answer_video_call(state: SharedWebRTCState, call_id: &str) -> Result<CallSession> {
    let mut s = state.write().await;
    let call = s
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;

    if call.status != CallStatus::Ringing {
        bail!("call {} is not in ringing state", call_id);
    }

    update_call_status(call, CallStatus::Connected);
    info!(call_id = %call_id, peer = %call.peer_onion, "answering call");

    let updated = call.clone();
    Ok(updated)
}

pub async fn end_video_call(state: SharedWebRTCState, call_id: &str) -> Result<()> {
    let mut s = state.write().await;
    let call = s
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;

    let ended_at = Utc::now();
    let duration_secs = (ended_at - call.started_at).num_seconds();

    update_call_status(call, CallStatus::Ended);
    let finished = call.clone();

    let entry = CallLogEntry {
        id: Uuid::new_v4().to_string(),
        peer_onion: finished.peer_onion.clone(),
        call_type: finished.call_type.clone(),
        direction: if finished.is_initiator { "outgoing" } else { "incoming" }.to_string(),
        started_at: finished.started_at,
        ended_at: Some(ended_at),
        duration_secs: Some(duration_secs),
        status: CallStatus::Ended,
    };
    s.call_history.push(entry);

    s.active_calls.remove(call_id);
    info!(call_id = %call_id, duration_secs = duration_secs, "call ended and logged");
    Ok(())
}

pub async fn start_screen_share(state: SharedWebRTCState, call_id: &str) -> Result<()> {
    let mut s = state.write().await;
    let call = s
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;

    if call.status != CallStatus::Connected {
        bail!("call {} is not connected", call_id);
    }

    call.screen_sharing = true;
    call.call_type = CallType::ScreenShare;
    update_call_status(call, CallStatus::ScreenSharing);
    info!(call_id = %call_id, "screen sharing started");

    Ok(())
}

pub async fn stop_screen_share(state: SharedWebRTCState, call_id: &str) -> Result<()> {
    let mut s = state.write().await;
    let call = s
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;

    call.screen_sharing = false;
    call.call_type = if call.video_enabled { CallType::Video } else { CallType::Audio };
    update_call_status(call, CallStatus::Connected);
    info!(call_id = %call_id, "screen sharing stopped");

    Ok(())
}

pub async fn toggle_audio_mute(state: SharedWebRTCState, call_id: &str) -> Result<bool> {
    let mut s = state.write().await;
    let call = s
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;

    call.audio_muted = !call.audio_muted;
    if call.audio_muted {
        update_call_status(call, CallStatus::Muted);
    } else {
        update_call_status(call, CallStatus::Connected);
    }

    info!(call_id = %call_id, muted = call.audio_muted, "audio mute toggled");
    Ok(call.audio_muted)
}

pub async fn toggle_video(state: SharedWebRTCState, call_id: &str) -> Result<bool> {
    let mut s = state.write().await;
    let call = s
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;

    call.video_enabled = !call.video_enabled;
    info!(call_id = %call_id, video = call.video_enabled, "video toggled");
    Ok(call.video_enabled)
}

pub async fn get_call_status(state: SharedWebRTCState, call_id: &str) -> Result<CallSession> {
    let s = state.read().await;
    let call = s
        .active_calls
        .get(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;
    Ok(call.clone())
}

pub async fn get_call_history(state: SharedWebRTCState) -> Vec<CallLogEntry> {
    let s = state.read().await;
    s.call_history.clone()
}

pub async fn get_active_calls(state: SharedWebRTCState) -> Vec<CallSession> {
    let s = state.read().await;
    s.active_calls.values().cloned().collect()
}

pub async fn add_ice_candidate(
    state: SharedWebRTCState,
    call_id: &str,
    candidate: &str,
) -> Result<()> {
    let mut s = state.write().await;
    let call = s
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;

    call.ice_candidates.push(candidate.to_string());
    debug!(call_id = %call_id, candidate = %candidate, "ICE candidate added");
    Ok(())
}

pub async fn send_call_reject(state: SharedWebRTCState, call_id: &str) -> Result<()> {
    let mut s = state.write().await;
    let call = s
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("call {} not found", call_id))?;

    let ended_at = Utc::now();
    let duration_secs = (ended_at - call.started_at).num_seconds();

    update_call_status(call, CallStatus::Rejected);
    let finished = call.clone();

    let entry = CallLogEntry {
        id: Uuid::new_v4().to_string(),
        peer_onion: finished.peer_onion.clone(),
        call_type: finished.call_type.clone(),
        direction: if finished.is_initiator { "outgoing" } else { "incoming" }.to_string(),
        started_at: finished.started_at,
        ended_at: Some(ended_at),
        duration_secs: Some(duration_secs),
        status: CallStatus::Rejected,
    };
    s.call_history.push(entry);

    s.active_calls.remove(call_id);
    info!(call_id = %call_id, "call rejected and logged");
    Ok(())
}

pub async fn cleanup_old_calls(state: SharedWebRTCState) -> Result<()> {
    let mut s = state.write().await;
    let now = Utc::now();
    let threshold = Duration::hours(1);

    let stale_ids: Vec<String> = s
        .active_calls
        .iter()
        .filter(|(_, c)| (now - c.started_at) > threshold)
        .map(|(id, _)| id.clone())
        .collect();

    for id in &stale_ids {
        if let Some(call) = s.active_calls.remove(id) {
            let ended_at = now;
            let duration_secs = (ended_at - call.started_at).num_seconds();
            warn!(call_id = %id, "cleaning up stale call");

            let entry = CallLogEntry {
                id: Uuid::new_v4().to_string(),
                peer_onion: call.peer_onion.clone(),
                call_type: call.call_type.clone(),
                direction: if call.is_initiator { "outgoing" } else { "incoming" }.to_string(),
                started_at: call.started_at,
                ended_at: Some(ended_at),
                duration_secs: Some(duration_secs),
                status: CallStatus::Failed,
            };
            s.call_history.push(entry);
        }
    }

    if !stale_ids.is_empty() {
        info!(count = stale_ids.len(), "cleaned up stale calls");
    }

    Ok(())
}
