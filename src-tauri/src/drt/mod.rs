use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: Vec<u8>, // 160-bit identifier (SHA-1 hash of public key)
    pub addresses: Vec<SocketAddr>,
    pub is_mobile: bool,
    pub last_seen: i64,
    pub state: NodeState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeState {
    Known,
    Connecting,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bucket {
    pub range_start: Vec<u8>,
    pub range_end: Vec<u8>,
    pub nodes: Vec<Node>,
    pub max_size: usize,
    pub connecting_count: usize,
}

impl Bucket {
    pub fn new(range_start: Vec<u8>, range_end: Vec<u8>, max_size: usize) -> Self {
        Self {
            range_start,
            range_end,
            nodes: Vec::new(),
            max_size,
            connecting_count: 0,
        }
    }

    pub fn add_node(&mut self, node: Node) -> bool {
        // Check if node already exists
        if let Some(pos) = self.nodes.iter().position(|n| n.id == node.id) {
            self.nodes[pos] = node;
            return true;
        }

        if self.nodes.len() < self.max_size {
            self.nodes.push(node);
            return true;
        }

        // Bucket full - if we're the closest bucket, try to split
        false
    }

    pub fn remove_node(&mut self, node_id: &[u8]) {
        self.nodes.retain(|n| n.id != node_id);
    }

    pub fn get_connected(&self) -> Vec<&Node> {
        self.nodes.iter().filter(|n| n.state == NodeState::Connected).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingTable {
    pub buckets: Vec<Bucket>,
    pub local_node_id: Vec<u8>,
    pub bucket_size: usize,
}

impl RoutingTable {
    pub fn new(local_node_id: Vec<u8>, bucket_size: usize) -> Self {
        // Create 160 buckets (one for each bit of the 160-bit ID space)
        let mut buckets = Vec::with_capacity(160);
        for i in 0..160 {
            let mut start = vec![0u8; 20];
            let mut end = vec![0u8; 20];
            start[i / 8] = 1 << (7 - (i % 8));
            end[i / 8] = (1 << (8 - (i % 8))) - 1;
            for j in (i / 8 + 1)..20 {
                end[j] = 255;
            }
            buckets.push(Bucket::new(start, end, 8)); // k=8
        }

        Self {
            buckets,
            local_node_id,
            bucket_size: 8,
        }
    }

    pub fn distance(&self, a: &[u8], b: &[u8]) -> Vec<u8> {
        a.iter().zip(b.iter()).map(|(a, b)| a ^ b).collect()
    }

    pub fn bucket_index(&self, target_id: &[u8]) -> usize {
        let dist = self.distance(&self.local_node_id, target_id);
        // Find first differing bit
        for (i, byte) in dist.iter().enumerate() {
            if *byte != 0 {
                let bit = byte.leading_zeros() as usize;
                return i * 8 + bit;
            }
        }
        159 // Max distance
    }

    pub fn add_node(&mut self, node: Node) -> bool {
        let idx = self.bucket_index(&node.id);
        if idx < self.buckets.len() {
            self.buckets[idx].add_node(node)
        } else {
            false
        }
    }

    pub fn find_closest(&self, target_id: &[u8], count: usize) -> Vec<Node> {
        let mut candidates = Vec::new();
        let idx = self.bucket_index(target_id);

        // Collect from closest buckets first
        for i in 0..self.buckets.len() {
            let bucket_idx = if i % 2 == 0 {
                idx.saturating_sub(i / 2)
            } else {
                idx + (i + 1) / 2
            };

            if bucket_idx < self.buckets.len() {
                for node in self.buckets[bucket_idx].get_connected() {
                    candidates.push(node.clone());
                    if candidates.len() >= count {
                        return candidates;
                    }
                }
            }
        }

        candidates
    }

    pub fn get_connected_nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::new();
        for bucket in &self.buckets {
            for node in bucket.get_connected() {
                nodes.push(node.clone());
            }
        }
        nodes
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DrpMessage {
    Find { query: Vec<u8>, num: u32, node_id: Vec<u8> },
    Found { query: Vec<u8>, nodes: Vec<Node>, mobile_nodes: Vec<Node> },
    Ping,
    Pong,
}

pub struct DrpClient {
    routing_table: Arc<RwLock<RoutingTable>>,
    local_id: Vec<u8>,
    transport: Arc<dyn DrpTransport>,
    buckets: Arc<RwLock<Vec<Bucket>>>,
}

impl DrpClient {
    pub fn new(local_id: Vec<u8>, transport: Arc<dyn DrpTransport>) -> Self {
        let routing_table = Arc::new(RwLock::new(RoutingTable::new(local_id.clone(), 8)));
        Self {
            routing_table: routing_table.clone(),
            local_id,
            transport,
            buckets: Arc::new(RwLock::new(routing_table.read().unwrap().buckets.clone())),
        }
    }

    pub async fn bootstrap(&self, bootstrap_nodes: Vec<Node>) -> Result<()> {
        for node in bootstrap_nodes {
            self.add_node(node).await?;
        }
        Ok(())
    }

    pub async fn add_node(&self, node: Node) -> Result<()> {
        let mut rt = self.routing_table.write().await;
        rt.add_node(node);
        Ok(())
    }

    pub async fn find_node(&self, target_id: &[u8]) -> Result<Vec<Node>> {
        let rt = self.routing_table.read().await;
        let closest = rt.find_closest(target_id, 8);
        Ok(closest)
    }

    pub async fn start(&self) -> Result<()> {
        // Periodic maintenance
        let rt = self.routing_table.clone();
        let transport = self.transport.clone();
        let local_id = self.local_id.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(600)); // 10 min
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                
                // Send FIND requests to maintain routing table
                let rt = rt.read().await;
                let connected = rt.get_connected_nodes();
                
                for node in connected {
                    let msg = DrpMessage::Find {
                        query: local_id.clone(),
                        num: 8,
                        node_id: node.id.clone(),
                    };
                    
                    if let Err(e) = transport.send(&node, msg).await {
                        debug!("Failed to send FIND to {}: {}", hex::encode(&node.id), e);
                    }
                }
            }
        });

        Ok(())
    }
}

#[async_trait::async_trait]
pub trait DrpTransport: Send + Sync {
    async fn send(&self, to: &Node, msg: DrpMessage) -> anyhow::Result<()>;
}

pub struct UdpDrpTransport {
    socket: std::sync::Arc<tokio::net::UdpSocket>,
}

impl UdpDrpTransport {
    pub fn new(socket: std::sync::Arc<tokio::net::UdpSocket>) -> Self {
        Self { socket }
    }
}

#[async_trait::async_trait]
impl DrpTransport for UdpDrpTransport {
    async fn send(&self, to: &Node, msg: DrpMessage) -> anyhow::Result<()> {
        let data = bincode::serialize(&msg)?;
        for addr in &to.addresses {
            self.socket.send_to(&data, addr).await?;
        }
        Ok(())
    }
}