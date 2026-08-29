pub mod kademlia;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::dht::kademlia::{Node, NodeId, StoredValue, Transport, UdpTransport};
use crate::crypto::keys::Ed25519KeyPair;

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

pub struct DhtClient {
    node: Arc<Node>,
    identity: Arc<RwLock<Option<String>>>,
    signing_key: Ed25519KeyPair,
    peers: Arc<RwLock<HashMap<String, PeerInfo>>>,
bootstrap_nodes: Vec<SocketAddr>,
    }

impl DhtClient {
    pub async fn new(bootstrap_nodes: Vec<SocketAddr>, signing_key: Ed25519KeyPair) -> Result<Self> {
        let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
        let transport: Arc<dyn Transport> = Arc::new(UdpTransport { socket: Arc::new(socket) });

        let node = Node::new(
            NodeId::random(),
            transport,
            bootstrap_nodes.clone(),
        );

        // If no explicit bootstrap nodes, this is truly serverless:
        // - We'll use local mDNS discovery on LAN
        // - Or wait for QR-code peer connections
        // - Or restore from persisted peer cache
        if bootstrap_nodes.is_empty() {
            tracing::info!("Starting in truly serverless mode: no bootstrap nodes configured");
        }
        
        // Only bootstrap if we have known nodes
        if !bootstrap_nodes.is_empty() {
            node.bootstrap().await?;
        }

        Ok(Self {
            node: Arc::new(node),
            identity: Arc::new(RwLock::new(None)),
            signing_key,
            peers: Arc::new(RwLock::new(HashMap::new())),
            bootstrap_nodes,
        })
    }

    pub async fn new_truly_serverless(signing_key: Ed25519KeyPair) -> Result<Self> {
        Self::new(vec![], signing_key).await
    }

    /// Bootstrap from a known peer's IP:port (e.g., from QR code or manual entry)
    pub async fn bootstrap_from_peer(&self, peer_addr: SocketAddr) -> Result<()> {
        self.node.bootstrap_from(peer_addr).await
    }

    /// Get currently known bootstrap peers from persisted cache
    pub async fn load_peer_cache(&self, cache_path: &std::path::Path) -> Result<()> {
        if let Ok(data) = tokio::fs::read(cache_path).await {
            if let Ok(peers) = serde_json::from_slice::<Vec<SocketAddr>>(&data) {
                for peer in peers {
                    let _ = self.bootstrap_from_peer(peer).await;
                }
            }
        }
        Ok(())
    }

    /// Save current routing table contacts to cache for next restart
    pub async fn save_peer_cache(&self, cache_path: &std::path::Path) -> Result<()> {
        let contacts = self.node.inner.table.lock().unwrap().buckets.iter().flatten().map(|c| c.addr).collect::<Vec<_>>();
        tokio::fs::write(cache_path, serde_json::to_vec(&contacts)?).await?;
        Ok(())
    }

    async fn start_presence_publisher(&self) {
        let node = self.node.clone();
        let identity = self.identity.clone();
        let signing_key = self.signing_key.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                if let Some(id) = identity.read().await.as_ref() {
                    if let Err(e) = Self::publish_presence(&node, &signing_key, id).await {
                        warn!("Failed to publish presence: {}", e);
                    }
                }
            }
        });
    }

    async fn publish_presence(node: &Arc<Node>, signing_key: &Ed25519KeyPair, identity: &str) -> Result<()> {
        let key = NodeId::from_sha256(format!("{}:presence", identity).as_bytes());
        let info = PeerInfo {
            identity: identity.to_string(),
            addresses: vec![],
            ice_candidates: vec![],
            last_seen: chrono::Utc::now().timestamp(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let value = StoredValue::new(key, serde_json::to_vec(&info)?, signing_key);

        node.put(&key, value).await?;
        debug!("Published presence for {}", identity);
        Ok(())
    }

    pub async fn find_peer(&self, peer_identity: &str) -> Result<Option<PeerInfo>> {
        let key = NodeId::from_sha256(format!("{}:presence", peer_identity).as_bytes());

        let values = self.node.get(&key).await?;
        for v in values {
            if let Ok(peer) = serde_json::from_slice::<PeerInfo>(&v.value) {
                return Ok(Some(peer));
            }
        }
        Ok(None)
    }

    pub async fn send_ice_candidates(&self, peer_identity: &str, candidates: Vec<IceCandidate>) -> Result<()> {
        let key = NodeId::from_sha256(format!("{}:ice", peer_identity).as_bytes());
        let msg = serde_json::to_vec(&candidates)?;

        let value = StoredValue::new(key, msg, &self.signing_key);

        self.node.put(&key, value).await?;

        Ok(())
    }

    pub async fn get_ice_candidates(&self, peer_identity: &str) -> Result<Vec<IceCandidate>> {
        let key = NodeId::from_sha256(format!("{}:ice", peer_identity).as_bytes());

        let values = self.node.get(&key).await?;
        for v in values {
            if let Ok(cands) = serde_json::from_slice::<Vec<IceCandidate>>(&v.value) {
                return Ok(cands);
            }
        }
        Ok(vec![])
    }

    pub async fn send_message(&self, peer_identity: &str, msg_type: &str, payload: &[u8]) -> Result<()> {
        let key = NodeId::from_sha256(format!("{}:msg", peer_identity).as_bytes());
        let value = StoredValue::new(key, payload.to_vec(), &self.signing_key);

        self.node.put(&key, value).await?;

        Ok(())
    }

    pub async fn get_messages(&self, peer_identity: &str) -> Result<Vec<Vec<u8>>> {
        let key = NodeId::from_sha256(format!("{}:msg", peer_identity).as_bytes());

        let values = self.node.get(&key).await?;
        Ok(values.into_iter().map(|v| v.value).collect())
    }
}