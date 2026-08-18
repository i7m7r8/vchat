use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error};

static WEBRTC_STATE: once_cell::sync::Lazy<Arc<RwLock<WebRTCState>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(WebRTCState::default())));

#[derive(Default)]
struct WebRTCState {
    active_calls: HashMap<String, CallSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallSession {
    pub call_id: String,
    pub peer_onion: String,
    pub is_initiator: bool,
    pub status: CallStatus,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CallStatus {
    Initiating,
    Ringing,
    Connected,
    Disconnected,
    Failed,
    Reconnecting,
}

pub async fn start_video_call(recipient_onion: &str) -> Result<String> {
    let call_id = uuid::Uuid::new_v4().to_string();

    let session = CallSession {
        call_id: call_id.clone(),
        peer_onion: recipient_onion.to_string(),
        is_initiator: true,
        status: CallStatus::Initiating,
        started_at: chrono::Utc::now().timestamp(),
    };

    {
        let mut state = WEBRTC_STATE.write().await;
        state.active_calls.insert(call_id.clone(), session);
    }

    info!("Initiating call to {recipient_onion}, call_id={call_id}");

    let peer = recipient_onion.to_string();
    let cid = call_id.clone();

    tokio::spawn(async move {
        establish_call_connection(&cid, &peer).await;
    });

    crate::error::audit_log(
        "call_initiated",
        &format!("peer={recipient_onion}, call_id={call_id}"),
    );

    Ok(call_id)
}

async fn establish_call_connection(call_id: &str, peer_onion: &str) {
    match crate::tor::connect_to_peer(peer_onion, 4434).await {
        Ok(mut stream) => {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let invite = serde_json::json!({
                "type": "call_invite",
                "call_id": call_id,
                "call_type": "video",
            });

            let invite_bytes = serde_json::to_vec(&invite).unwrap_or_default();
            if let Err(e) = stream.write_all(&invite_bytes).await {
                error!("Failed to send call invite: {e}");
                update_call_status(call_id, CallStatus::Failed).await;
                return;
            }

            let mut buf = vec![0u8; 4096];
            match stream.read(&mut buf).await {
                Ok(0) => {
                    warn!("Call invite response empty");
                    update_call_status(call_id, CallStatus::Failed).await;
                }
                Ok(_n) => {
                    info!("Call accepted by {peer_onion}");
                    update_call_status(call_id, CallStatus::Connected).await;
                }
                Err(e) => {
                    error!("Call connection error: {e}");
                    update_call_status(call_id, CallStatus::Failed).await;
                }
            }
        }
        Err(e) => {
            warn!("Cannot reach peer {peer_onion} for call: {e}");
            update_call_status(call_id, CallStatus::Failed).await;
        }
    }
}

async fn update_call_status(call_id: &str, status: CallStatus) {
    let mut state = WEBRTC_STATE.write().await;
    if let Some(call) = state.active_calls.get_mut(call_id) {
        call.status = status;
    }
}

pub async fn answer_video_call(call_id: &str) -> Result<()> {
    let mut state = WEBRTC_STATE.write().await;
    let call = state
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("Call not found: {call_id}"))?;

    call.status = CallStatus::Connected;
    info!("Call answered: {call_id}");
    Ok(())
}

pub async fn end_video_call(call_id: &str) -> Result<()> {
    let mut state = WEBRTC_STATE.write().await;
    if let Some(_call) = state.active_calls.remove(call_id) {
        info!("Call ended: {call_id}");
        crate::error::audit_log("call_ended", &format!("call_id={call_id}"));
    }
    Ok(())
}

pub async fn start_screen_share(call_id: &str) -> Result<()> {
    let mut state = WEBRTC_STATE.write().await;
    let call = state
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("Call not found: {call_id}"))?;

    if call.status != CallStatus::Connected {
        anyhow::bail!("Call not connected");
    }

    info!("Screen sharing started for call {call_id}");
    Ok(())
}

pub async fn stop_screen_share(call_id: &str) -> Result<()> {
    info!("Screen sharing stopped for call {call_id}");
    Ok(())
}

pub async fn get_call_status(call_id: &str) -> Option<CallStatus> {
    let state = WEBRTC_STATE.read().await;
    state.active_calls.get(call_id).map(|c| c.status.clone())
}
