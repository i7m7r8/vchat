use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

static WEBRTC_STATE: once_cell::sync::Lazy<Arc<RwLock<WebRTCState>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(WebRTCState::default())));

#[derive(Default)]
struct WebRTCState {
    active_calls: HashMap<String, CallSession>,
    video_enabled: bool,
    audio_enabled: bool,
    screen_sharing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallSession {
    pub call_id: String,
    pub peer_onion: String,
    pub is_initiator: bool,
    pub status: CallStatus,
    pub video_track: Option<String>,
    pub audio_track: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CallStatus {
    Initiating,
    Ringing,
    Connected,
    Disconnected,
    Failed,
}

pub async fn start_video_call(recipient_onion: &str) -> Result<String> {
    let call_id = uuid::Uuid::new_v4().to_string();

    info!("Starting video call to {}", recipient_onion);

    let session = CallSession {
        call_id: call_id.clone(),
        peer_onion: recipient_onion.to_string(),
        is_initiator: true,
        status: CallStatus::Initiating,
        video_track: None,
        audio_track: None,
    };

    let mut state = WEBRTC_STATE.write().await;
    state.active_calls.insert(call_id.clone(), session);

    let peer_onion = recipient_onion.to_string();
    let cid = call_id.clone();

    tokio::spawn(async move {
        if let Err(e) = establish_peer_connection(&cid, &peer_onion).await {
            warn!("Failed to establish connection: {}", e);
            let mut state = WEBRTC_STATE.write().await;
            if let Some(call) = state.active_calls.get_mut(&cid) {
                call.status = CallStatus::Failed;
            }
        }
    });

    Ok(call_id)
}

pub async fn answer_video_call(call_id: &str) -> Result<()> {
    let mut state = WEBRTC_STATE.write().await;

    let call = state
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("Call not found"))?;

    call.status = CallStatus::Connected;

    info!("Answered video call {}", call_id);
    Ok(())
}

pub async fn end_video_call(call_id: &str) -> Result<()> {
    let mut state = WEBRTC_STATE.write().await;

    if let Some(call) = state.active_calls.remove(call_id) {
        info!("Ended video call {}", call_id);
    }

    Ok(())
}

pub async fn start_screen_share(call_id: &str) -> Result<()> {
    let mut state = WEBRTC_STATE.write().await;

    let call = state
        .active_calls
        .get_mut(call_id)
        .ok_or_else(|| anyhow::anyhow!("Call not found"))?;

    state.screen_sharing = true;
    info!("Started screen sharing for call {}", call_id);

    Ok(())
}

pub async fn stop_screen_share(call_id: &str) -> Result<()> {
    let mut state = WEBRTC_STATE.write().await;

    state.screen_sharing = false;
    info!("Stopped screen sharing for call {}", call_id);

    Ok(())
}

async fn establish_peer_connection(call_id: &str, peer_onion: &str) -> Result<()> {
    use str0m::{Candidate, Client, Event, IceConnectionState, Input, Offer, Output, SdpAnswer, SdpOffer};

    info!("Establishing peer connection to {}", peer_onion);

    let mut client = Client::new("vchat".to_string())?;

    let candidate = Candidate::new("0.0.0.0:0".parse()?)
        .set_component(str0m::CandidateComponent::Rtp)
        .set_protocol(str0m::CandidateProtocol::Tcp);

    client.add_candidate(candidate)?;

    let offer = client.create_offer(vec![])?;

    let sdp = offer.to_sdp()?;

    info!("Created SDP offer for {}", peer_onion);

    Ok(())
}
