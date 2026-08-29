use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub mod ice;
pub use ice::{check_connectivity, IceCandidate, RelayConfig};
pub mod srtp;
pub mod media;
pub mod conference;
pub use srtp::{SrtpContext, SrtpProfile};

/// A per-call UDP media transport session (Jami-style ICE/UDP).
pub struct MediaSession {
    pub socket: Arc<UdpSocket>,
    pub local: std::net::SocketAddr,
    pub local_candidates: Vec<IceCandidate>,
    pub remote_candidates: Vec<IceCandidate>,
    pub remote: Option<std::net::SocketAddr>,
    pub connected: bool,
    pub srtp_tx: Option<SrtpContext>,  // Outbound SRTP context
    pub srtp_rx: Option<SrtpContext>,  // Inbound SRTP context
}

#[derive(Debug, Clone)]
pub struct MediaSessionInfo {
    pub local_addr: String,
    pub local_candidates: Vec<String>,
    pub connected: bool,
}

#[derive(Debug, Clone)]
pub enum MediaFrameKind {
    Video,
    Screen,
    Voice,
}

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
    pub media_sessions: HashMap<String, MediaSession>,
}

impl WebRTCState {
    pub fn new() -> Self {
        Self {
            active_calls: HashMap::new(),
            call_history: Vec::new(),
            media_sessions: HashMap::new(),
        }
    }
}

impl Default for WebRTCState {
    fn default() -> Self {
        Self::new()
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

pub async fn establish_incoming_call(
    state: SharedWebRTCState,
    call_id: &str,
    peer_onion: &str,
    call_type: CallType,
) -> Result<CallSession> {
    let mut session = build_session(call_id, peer_onion, false, call_type.clone());
    if call_type == CallType::Audio {
        session.video_enabled = false;
    }
    session.status = CallStatus::Ringing;

    info!(call_id = %call_id, peer = %peer_onion, "incoming call created (ringing)");

    let mut s = state.write().await;
    s.active_calls.insert(call_id.to_string(), session.clone());
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
        s.media_sessions.remove(id);
    }

    if !stale_ids.is_empty() {
        info!(count = stale_ids.len(), "cleaned up stale calls");
    }

    Ok(())
}

// ── Jami-style ICE/UDP media transport ──────────────────────────────────────

/// Bind a UDP socket for the call and gather ICE candidates (host, plus
/// server-reflexive and relayed when a self-hostable STUN/TURN is configured).
pub async fn init_media_session(
    state: SharedWebRTCState,
    call_id: &str,
    relay: RelayConfig,
    app: tauri::AppHandle,
) -> Result<MediaSessionInfo> {
    let mut s = state.write().await;
    if s.media_sessions.contains_key(call_id) {
        let existing = s.media_sessions.get(call_id).unwrap();
        return Ok(MediaSessionInfo {
            local_addr: existing.local.to_string(),
            local_candidates: existing.local_candidates.iter().map(|c| c.to_string()).collect(),
            connected: existing.connected,
        });
    }

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let local = socket.local_addr()?;
    let socket = Arc::new(socket);

    let local_candidates = ice::gather_candidates(&socket, &relay).await;

    let session = MediaSession {
        socket: socket.clone(),
        local,
        local_candidates: local_candidates.clone(),
        remote_candidates: Vec::new(),
        remote: None,
        connected: false,
    };
    s.media_sessions.insert(call_id.to_string(), session);
    drop(s);

    // Initialize SRTP contexts when the session connects (will be set via set_media_key)
    let session = MediaSession {
        socket: socket.clone(),
        local,
        local_candidates: local_candidates.clone(),
        remote_candidates: Vec::new(),
        remote: None,
        connected: false,
        srtp_tx: None,
        srtp_rx: None,
    };
    s.media_sessions.insert(call_id.to_string(), session);
    drop(s);

    spawn_media_receiver(state.clone(), call_id.to_string(), socket, app);

    Ok(MediaSessionInfo {
        local_addr: local.to_string(),
        local_candidates: local_candidates.iter().map(|c| c.to_string()).collect(),
        connected: false,
    })
}

/// Background task that reassembles chunked UDP media frames and emits them to
/// the frontend as `udp-media-frame` events (Jami-style direct media path).
fn spawn_media_receiver(
    state: SharedWebRTCState,
    call_id: String,
    socket: Arc<UdpSocket>,
    app: tauri::AppHandle,
) {
    tauri::async_runtime::spawn(async move {
        // Partial frame reassembly buffers, keyed by frame_id.
        let mut partials: HashMap<u32, (u32, std::collections::BTreeMap<u32, Vec<u8>>)> =
            HashMap::new();
        let mut last_frame = 0u32;
        let mut buf = vec![0u8; 4096];
        let mut srtp_rx: Option<SrtpContext> = None;
        loop {
            let n = match socket.recv(&mut buf).await {
                Ok(n) => n,
                Err(_) => break,
            };
            if !is_active(&state, &call_id).await {
                break;
            }
            let Some(chunk) = ice::decode_chunk(&buf[..n]) else {
                continue;
            };
            let entry = partials
                .entry(chunk.frame_id)
                .or_insert((chunk.total, std::collections::BTreeMap::new()));
            entry.1.insert(chunk.offset, chunk.data);

            // Reassemble when all pieces are present (skip zero-length frames).
            if chunk.total == 0 {
                continue;
            }
            let (total, pieces) = entry;
            let got: u32 = pieces.values().map(|p| p.len() as u32).sum();
            if got == *total {
                let mut frame = Vec::with_capacity(*total as usize);
                for piece in pieces.values() {
                    frame.extend_from_slice(piece);
                }
                partials.remove(&chunk.frame_id);
                
                // Unprotect with SRTP if available
                let unprotected = if let Some(srtp) = &mut srtp_rx {
                    srtp.unprotect_rtp(&frame).unwrap_or(frame)
                } else {
                    frame
                };
                
                if chunk.frame_id > last_frame || last_frame == 0 {
                    last_frame = chunk.frame_id;
                }
                if let Err(e) = app.emit("udp-media-frame", serde_json::json!({
                    "call_id": call_id,
                    "frame_id": chunk.frame_id,
                    "data": unprotected,
                })) {
                    warn!("Failed to emit udp-media-frame: {e}");
                }
                if partials.len() > 64 {
                    partials.clear();
                }
            }
        }
    });
}

async fn is_active(state: &SharedWebRTCState, call_id: &str) -> bool {
    let s = state.read().await;
    s.media_sessions.contains_key(call_id)
}

pub async fn get_media_session(state: SharedWebRTCState, call_id: &str) -> Result<MediaSessionInfo> {
    let s = state.read().await;
    let ms = s
        .media_sessions
        .get(call_id)
        .ok_or_else(|| anyhow::anyhow!("no media session for {call_id}"))?;
    Ok(MediaSessionInfo {
        local_addr: ms.local.to_string(),
        local_candidates: ms.local_candidates.iter().map(|c| c.to_string()).collect(),
        connected: ms.connected,
    })
}

/// Record a remote candidate received over the Tor signaling channel.
pub async fn add_remote_ice_candidate(
    state: SharedWebRTCState,
    call_id: &str,
    candidate: &str,
) -> Result<()> {
    let Some(cand) = ice::parse_candidate(candidate) else {
        warn!(candidate = %candidate, "ignoring malformed ICE candidate");
        return Ok(());
    };
    let mut s = state.write().await;
    let ms = s
        .media_sessions
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("no media session for {call_id}"))?;
    if !ms.remote_candidates.iter().any(|c| c.addr == cand.addr) {
        ms.remote_candidates.push(cand);
    }
    Ok(())
}

/// Run ICE connectivity checks against the remote candidates and remember the
/// working peer address so media can be sent over UDP.
pub async fn run_ice_connect(
    state: SharedWebRTCState,
    call_id: &str,
) -> Result<bool> {
    let mut s = state.write().await;
    let ms = s
        .media_sessions
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("no media session for {call_id}"))?;

    if ms.connected {
        return Ok(true);
    }

    // Prefer srflx/relay pairs from the remote, then host.
    let mut ordered: Vec<IceCandidate> = ms.remote_candidates.clone();
    ordered.sort_by_key(|c| std::cmp::Reverse(c.priority));

    for cand in ordered {
        let check = check_connectivity(&ms.socket, cand.addr).await;
        if check {
            ms.remote = Some(cand.addr);
            ms.connected = true;
            info!(call_id = %call_id, peer = %cand.addr, "ICE connectivity established via {}", cand.candidate_type_str());
            return Ok(true);
        }
    }
    debug!(call_id = %call_id, "ICE connectivity checks failed for all remote candidates");
    Ok(false)
}

impl IceCandidate {
    fn candidate_type_str(&self) -> &'static str {
        match self.candidate_type {
            ice::CandidateType::Host => "host",
            ice::CandidateType::Srflx => "srflx",
            ice::CandidateType::Relay => "relay",
        }
    }
}

/// Send media data over the established UDP path with SRTP protection.
/// Returns false if no UDP path is connected yet (caller should fall back to the Tor path).
pub async fn send_media_frame(
    state: SharedWebRTCState,
    call_id: &str,
    frame_id: u32,
    data: &[u8],
) -> Result<bool> {
    let mut s = state.write().await;
    let ms = s
        .media_sessions
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("no media session for {call_id}"))?;
    let Some(remote) = ms.remote else {
        return Ok(false);
    };
    
    let mut protected = if let Some(srtp) = &mut ms.srtp_tx {
        srtp.protect_rtp(data)
    } else {
        data.to_vec()
    };
    
    ice::send_frame(&ms.socket, remote, frame_id, &protected).await?;
    Ok(true)
}

/// Tear down the UDP media session for a call.
pub async fn close_media_session(state: SharedWebRTCState, call_id: &str) {
    let mut s = state.write().await;
    s.media_sessions.remove(call_id);
}

/// Set the SRTP session key for a call (called after X3DH ratchet establishes a shared key).
/// `shared_secret` should be a 32-byte key from the ratchet.
/// `direction` 0 = outbound (we send), 1 = inbound (we receive).
/// Uses AES-256-CTR-HMAC-SHA1-80 profile by default.
pub async fn set_media_key(
    state: SharedWebRTCState,
    call_id: &str,
    shared_secret: &[u8; 32],
    ssrc: u32,
) -> Result<()> {
    let mut s = state.write().await;
    let ms = s
        .media_sessions
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("no media session for {call_id}"))?;
    
    // Create outbound (direction 0) and inbound (direction 1) contexts
    let profile = SrtpProfile::Aes256CtrHmacSha1_80;
    ms.srtp_tx = Some(SrtpContext::from_shared_secret(shared_secret, 0, ssrc, profile));
    ms.srtp_rx = Some(SrtpContext::from_shared_secret(shared_secret, 1, ssrc, profile));
    
    info!(call_id = %call_id, "SRTP session keys initialized");
    Ok(())
}
