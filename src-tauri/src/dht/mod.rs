use anyhow::{Context, Result};
use opendht::{
    crypto::identity::Id,
    dht::DhtRunner,
    info::Value,
    Distance,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub identity: String,
    pub addresses: Vec<SocketAddr>,
    pub ice_candidates: Vec<IceCandidate>,
    pub last_seen: i64,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceCandidate {
    pub candidate_type: String, // "host", "srflx", "relay"
    pub address: String,
    pub port: u16,
    pub priority: u32,
    pub protocol: String, // "udp", "tcp"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtMessage {
    pub from: String,
    pub to: String,
    pub message_type: String,
    pub payload: Vec<u8>,
    pub timestamp: i64,
}

pub struct DhtClient {
    dht: Arc<DhtRunner>,
    identity: Arc<RwLock<Option<String>>>,
    peers: Arc<RwLock<HashMap<String, PeerInfo>>>,
    bootstrap_nodes: Vec<String>,
}

impl DhtClient {
    pub async fn new(bootstrap_nodes: Vec<String>) -> Result<Self> {
        let dht = DhtRunner::new().context("Failed to create DHT runner")?;

        // Start DHT with bootstrap nodes
        for node in &bootstrap_nodes {
            dht.bootstrap(node).context("Failed to bootstrap DHT")?;
        }

        // Wait for bootstrap to complete
        tokio::time::sleep(Duration::from_secs(3)).await;

        Ok(Self {
            dht: Arc::new(dht),
            identity: Arc::new(RwLock::new(None)),
            peers: Arc::new(RwLock::new(HashMap::new())),
            bootstrap_nodes,
        })
    }

    pub async fn set_identity(&self, identity: String) {
        *self.identity.write().await = Some(identity);
    }

    pub async fn start(&self) -> Result<()> {
        // Start listening for incoming connections
        self.dht.listen_on("0.0.0.0:0").context("Failed to listen on DHT")?;

        // Start periodic presence publishing
        self.start_presence_publisher().await;

        // Start peer discovery
        self.start_peer_discovery().await;

        Ok(())
    }

    async fn start_presence_publisher(&self) {
        let dht = self.dht.clone();
        let identity = self.identity.clone();
        let peers = self.peers.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                if let Some(id) = identity.read().await.as_ref() {
                    if let Err(e) = Self::publish_presence(&dht, id).await {
                        warn!("Failed to publish presence: {}", e);
                    }
                }
            }
        });
    }

    async fn publish_presence(dht: &Arc<DhtRunner>, identity: &str) -> Result<()> {
        let info = PeerInfo {
            identity: identity.to_string(),
            addresses: vec![], // Will be populated by ICE
            ice_candidates: vec![],
            last_seen: chrono::Utc::now().timestamp(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let key = format!("{}:presence", identity);
        let value = serde_json::to_vec(&info)?;
        let dht_key = Id::new(key.as_bytes());

        dht.put(dht_key, Value::new(value)).context("Failed to put presence")?;
        debug!("Published presence for {}", identity);
        Ok(())
    }

    async fn start_peer_discovery(&self) {
        let dht = self.dht.clone();
        let peers = self.peers.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                // Peer discovery logic here
            }
        });
    }

    pub async fn find_peer(&self, peer_identity: &str) -> Result<Option<PeerInfo>> {
        let key = format!("{}:presence", peer_identity);
        let dht_key = Id::new(key.as_bytes());

        match self.dht.get(dht_key).await {
            Ok(values) => {
                if let Some(value) = values.first() {
                    let peer: PeerInfo = serde_json::from_slice(value.data())?;
                    Ok(Some(peer))
                } else {
                    Ok(None)
                }
            }
            Err(e) => {
                debug!("Peer {} not found: {}", peer_identity, e);
                Ok(None)
            }
        }
    }

    pub async fn send_ice_candidates(&self, peer_identity: &str, candidates: Vec<IceCandidate>) -> Result<()> {
        let key = format!("{}:ice", peer_identity);
        let dht_key = Id::new(key.as_bytes());

        let msg = DhtMessage {
            from: self.identity.read().await.clone().unwrap_or_default(),
            to: peer_identity.to_string(),
            message_type: "ice_candidates".to_string(),
            payload: serde_json::to_vec(&candidates)?,
            timestamp: chrono::Utc::now().timestamp(),
        };

        let value = serde_json::to_vec(&msg)?;
        self.dht.put(dht_key, Value::new(value)).context("Failed to put ICE candidates")?;

        Ok(())
    }

    pub async fn get_ice_candidates(&self, peer_identity: &str) -> Result<Vec<IceCandidate>> {
        let key = format!("{}:ice", peer_identity);
        let dht_key = Id::new(key.as_bytes());

        match self.dht.get(dht_key).await {
            Ok(values) => {
                if let Some(value) = values.first() {
                    let msg: DhtMessage = serde_json::from_slice(value.data())?;
                    let candidates: Vec<IceCandidate> = serde_json::from_slice(&msg.payload)?;
                    Ok(candidates)
                } else {
                    Ok(vec![])
                }
            }
            Err(_) => Ok(vec![]),
        }
    }

    pub async fn send_message(&self, peer_identity: &str, msg_type: &str, payload: &[u8]) -> Result<()> {
        let key = format!("{}:msg", peer_identity);
        let dht_key = Id::new(key.as_bytes());

        let msg = DhtMessage {
            from: self.identity.read().await.clone().unwrap_or_default(),
            to: peer_identity.to_string(),
            message_type: msg_type.to_string(),
            payload: payload.to_vec(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        let value = serde_json::to_vec(&msg)?;
        self.dht.put(dht_key, Value::new(value)).context("Failed to put message")?;

        Ok(())
    }
}

impl Default for DhtClient {
    fn default() -> Self {
        // Default bootstrap nodes (OpenDHT public bootstrap)
        let bootstrap_nodes = vec![
            "bootstrap.jami.net:4222".to_string(),
            "bootstrap2.jami.net:4222".to_string(),
        ];
        // Note: actual initialization happens via new()
        unimplemented!("Use DhtClient::new() instead")
    }
}