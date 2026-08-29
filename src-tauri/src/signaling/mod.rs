use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallType {
    Audio,
    Video,
    ScreenShare,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallState {
    Idle,
    Calling,
    Ringing,
    Connected,
    OnHold,
    Ended,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallSession {
    pub call_id: String,
    pub peer_identity: String,
    pub call_type: CallType,
    pub state: CallState,
    pub is_initiator: bool,
    pub created_at: i64,
    pub connected_at: Option<i64>,
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub ice_candidates_local: Vec<String>,
    pub ice_candidates_remote: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignalingMessage {
    Invite {
        call_id: String,
        call_type: CallType,
        sdp: String,
        ice_candidates: Vec<String>,
    },
    Ringing {
        call_id: String,
    },
    Accept {
        call_id: String,
        sdp: String,
        ice_candidates: Vec<String>,
    },
    Reject {
        call_id: String,
        reason: String,
    },
    End {
        call_id: String,
    },
    IceCandidate {
        call_id: String,
        candidate: String,
    },
    MediaChange {
        call_id: String,
        sdp: String,
    },
    Heartbeat {
        call_id: String,
    },
}

pub struct SignalingClient {
    transport: Arc<dyn Transport>,
    calls: Arc<RwLock<HashMap<String, CallSession>>>,
    pending_invites: Arc<RwLock<HashMap<String, mpsc::Sender<SignalingMessage>>>>,
    local_identity: String,
}

impl SignalingClient {
    pub fn new(transport: Arc<dyn Transport>, local_identity: String) -> Self {
        Self {
            transport,
            calls: Arc::new(RwLock::new(HashMap::new())),
            pending_invites: Arc::new(RwLock::new(HashMap::new())),
            local_identity,
        }
    }

    pub async fn start(&self) -> Result<()> {
        // Start listening for incoming messages
        self.start_listener().await;
        Ok(())
    }

    pub async fn invite(
        &self,
        peer_identity: &str,
        call_type: CallType,
        local_sdp: String,
        ice_candidates: Vec<String>,
    ) -> Result<String> {
        let call_id = Uuid::new_v4().to_string();
        let call = CallSession {
            call_id: call_id.clone(),
            peer_identity: peer_identity.to_string(),
            call_type: call_type.clone(),
            state: CallState::Calling,
            is_initiator: true,
            created_at: chrono::Utc::now().timestamp(),
            connected_at: None,
            local_sdp: Some(local_sdp.clone()),
            remote_sdp: None,
            ice_candidates_local: ice_candidates.clone(),
            ice_candidates_remote: vec![],
        };

        self.calls.write().await.insert(call_id.clone(), call);

        let msg = SignalingMessage::Invite {
            call_id: call_id.clone(),
            call_type,
            sdp: local_sdp,
            ice_candidates,
        };

        self.transport.send(peer_identity, &msg).await?;
        info!("Sent INVITE to {} for call {}", peer_identity, call_id);

        // Set up timeout for call acceptance
        let calls = self.calls.clone();
        let call_id_clone = call_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let mut calls = calls.write().await;
            if let Some(call) = calls.get_mut(&call_id_clone) {
                if call.state == CallState::Calling || call.state == CallState::Ringing {
                    call.state = CallState::Failed;
                    warn!("Call {} timed out", call_id_clone);
                }
            }
        });

        Ok(call_id)
    }

    pub async fn accept(
        &self,
        call_id: &str,
        local_sdp: String,
        ice_candidates: Vec<String>,
    ) -> Result<()> {
        let mut calls = self.calls.write().await;
        let call = calls.get_mut(call_id)
            .ok_or_else(|| anyhow::anyhow!("Call not found"))?;

        call.state = CallState::Connected;
        call.connected_at = Some(chrono::Utc::now().timestamp());
        call.local_sdp = Some(local_sdp.clone());
        call.ice_candidates_local = ice_candidates.clone();

        let msg = SignalingMessage::Accept {
            call_id: call_id.to_string(),
            sdp: local_sdp,
            ice_candidates,
        };

        self.transport.send(&call.peer_identity, &msg).await?;
        info!("Accepted call {}", call_id);
        Ok(())
    }

    pub async fn reject(&self, call_id: &str, reason: String) -> Result<()> {
        let mut calls = self.calls.write().await;
        let call = calls.get_mut(call_id)
            .ok_or_else(|| anyhow::anyhow!("Call not found"))?;

        let peer = call.peer_identity.clone();
        call.state = CallState::Ended;

        let msg = SignalingMessage::Reject {
            call_id: call_id.to_string(),
            reason,
        };

        self.transport.send(&peer, &msg).await?;
        calls.remove(call_id);
        info!("Rejected call {}", call_id);
        Ok(())
    }

    pub async fn end_call(&self, call_id: &str) -> Result<()> {
        let mut calls = self.calls.write().await;
        let call = calls.get_mut(call_id)
            .ok_or_else(|| anyhow::anyhow!("Call not found"))?;

        let peer = call.peer_identity.clone();
        call.state = CallState::Ended;

        let msg = SignalingMessage::End {
            call_id: call_id.to_string(),
        };

        self.transport.send(&peer, &msg).await?;
        calls.remove(call_id);
        info!("Ended call {}", call_id);
        Ok(())
    }

    pub async fn add_ice_candidate(&self, call_id: &str, candidate: String) -> Result<()> {
        let mut calls = self.calls.write().await;
        let call = calls.get_mut(call_id)
            .ok_or_else(|| anyhow::anyhow!("Call not found"))?;

        call.ice_candidates_local.push(candidate.clone());

        let msg = SignalingMessage::IceCandidate {
            call_id: call_id.to_string(),
            candidate,
        };

        self.transport.send(&call.peer_identity, &msg).await?;
        Ok(())
    }

    pub async fn handle_incoming(&self, from: String, msg: SignalingMessage) -> Result<()> {
        match msg {
            SignalingMessage::Invite { call_id, call_type, sdp, ice_candidates } => {
                let call = CallSession {
                    call_id: call_id.clone(),
                    peer_identity: from.clone(),
                    call_type,
                    state: CallState::Ringing,
                    is_initiator: false,
                    created_at: chrono::Utc::now().timestamp(),
                    connected_at: None,
                    local_sdp: None,
                    remote_sdp: Some(sdp),
                    ice_candidates_local: vec![],
                    ice_candidates_remote: ice_candidates,
                };

                self.calls.write().await.insert(call_id.clone(), call);
                info!("Incoming call {} from {}", call_id, from);

                // Emit event for UI to show incoming call
                // This would be connected to the frontend
            }
            SignalingMessage::Ringing { call_id } => {
                if let Some(call) = self.calls.write().await.get_mut(&call_id) {
                    call.state = CallState::Ringing;
                }
            }
            SignalingMessage::Accept { call_id, sdp, ice_candidates } => {
                if let Some(call) = self.calls.write().await.get_mut(&call_id) {
                    call.state = CallState::Connected;
                    call.connected_at = Some(chrono::Utc::now().timestamp());
                    call.remote_sdp = Some(sdp);
                    call.ice_candidates_remote = ice_candidates;
                    info!("Call {} accepted", call_id);
                }
            }
            SignalingMessage::Reject { call_id, reason } => {
                let mut calls = self.calls.write().await;
                if let Some(call) = calls.remove(&call_id) {
                    info!("Call {} rejected: {}", call_id, reason);
                }
            }
            SignalingMessage::End { call_id } => {
                let mut calls = self.calls.write().await;
                if let Some(call) = calls.remove(&call_id) {
                    info!("Call {} ended", call_id);
                }
            }
            SignalingMessage::IceCandidate { call_id, candidate } => {
                if let Some(call) = self.calls.write().await.get_mut(&call_id) {
                    call.ice_candidates_remote.push(candidate);
                }
            }
            SignalingMessage::MediaChange { call_id, sdp } => {
                if let Some(call) = self.calls.write().await.get_mut(&call_id) {
                    call.remote_sdp = Some(sdp);
                    // Renegotiate media
                }
            }
            SignalingMessage::Heartbeat { call_id } => {
                // Keep-alive for NAT
            }
        }
        Ok(())
    }

    async fn start_listener(&self) {
        // This would be connected to the transport layer
        // For now, we just set up the message handler
    }

    pub async fn get_call(&self, call_id: &str) -> Option<CallSession> {
        self.calls.read().await.get(call_id).cloned()
    }

    pub async fn list_calls(&self) -> Vec<CallSession> {
        self.calls.read().await.values().cloned().collect()
    }
}

#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, to: &str, msg: &SignalingMessage) -> Result<()>;
    async fn start_listening(&self, handler: Arc<dyn Fn(String, SignalingMessage) + Send + Sync>) -> Result<()>;
}

pub struct DhtTransport {
    dht_client: Arc<dht::DhtClient>,
    handler: Option<Arc<dyn Fn(String, SignalingMessage) + Send + Sync>>,
}

impl DhtTransport {
    pub fn new(dht_client: Arc<dht::DhtClient>) -> Self {
        Self {
            dht_client,
            handler: None,
        }
    }
}

#[async_trait::async_trait]
impl Transport for DhtTransport {
    async fn send(&self, to: &str, msg: &SignalingMessage) -> Result<()> {
        let data = serde_json::to_vec(msg)?;
        self.dht_client.send_message(to, "signaling", &data).await
    }

    async fn start_listening(&self, handler: Arc<dyn Fn(String, SignalingMessage) + Send + Sync>) -> Result<()> {
        // Set up DHT listener for incoming messages
        // This would poll the DHT for messages addressed to us
        Ok(())
    }
}