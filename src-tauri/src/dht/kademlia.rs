//! Pure-Rust Kademlia distributed hash table.
//!
//! A self-contained DHT engine with no native dependencies. Node IDs are
//! 256-bit (SHA-256 of an identity's public key). Routing uses k-buckets
//! (k=8, 256 buckets) keyed by XOR distance. The real builder can swap the
//! internal engine for native OpenDHT behind a compatible trait later.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::crypto::keys::Ed25519KeyPair;

/// ID space width in bits.
pub const ID_BITS: usize = 256;
/// Bucket size.
pub const K: usize = 8;
/// Concurrency factor (parallel queries per lookup round).
pub const ALPHA: usize = 3;
/// Maximum lookup rounds before giving up.
pub const MAX_ROUNDS: usize = 6;
/// Per-request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(800);
/// How long a stored value is considered fresh.
const VALUE_TTL: Duration = Duration::from_secs(300);

/// A 256-bit node / key identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    /// Hash arbitrary bytes into a node id.
    pub fn from_sha256(data: &[u8]) -> Self {
        let mut h = sha2::Sha256::new();
        h.update(data);
        let out = h.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&out);
        Self(arr)
    }

    /// Node id derived from an identity's ed25519 public key.
    pub fn from_pubkey(pubkey: &[u8; 32]) -> Self {
        Self::from_sha256(pubkey)
    }

    /// Random id (used before an identity is provisioned).
    pub fn random() -> Self {
        Self(rand::random())
    }

    /// XOR distance to another id (the Kademlia metric).
    pub fn distance(&self, other: &NodeId) -> Distance {
        let mut d = [0u8; 32];
        for i in 0..32 {
            d[i] = self.0[i] ^ other.0[i];
        }
        Distance(d)
    }
}

/// XOR distance treated as a big-endian number so ordering by bytes matches
/// ordering by numeric XOR distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Distance(pub [u8; 32]);

impl PartialOrd for Distance {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Distance {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

/// A known reachable node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Contact {
    pub id: NodeId,
    pub addr: SocketAddr,
}

/// A signed, time-stamped key-value record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredValue {
    pub key: NodeId,
    pub value: Vec<u8>,
    /// ed25519 public key of the signer / owner.
    pub pubkey: [u8; 32],
    pub signature: Vec<u8>,
    pub timestamp: i64,
}

fn sign_payload(key: &NodeId, value: &[u8], timestamp: i64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32 + value.len() + 8);
    buf.extend_from_slice(&key.0);
    buf.extend_from_slice(value);
    buf.extend_from_slice(&timestamp.to_le_bytes());
    buf
}

impl StoredValue {
    /// Build a value signed by `signing_key` for integrity.
    pub fn new(key: NodeId, value: Vec<u8>, signing_key: &Ed25519KeyPair) -> Self {
        let timestamp = chrono::Utc::now().timestamp();
        let sig = signing_key.sign(&sign_payload(&key, &value, timestamp));
        Self {
            key,
            value,
            pubkey: signing_key.public_key_bytes(),
            signature: sig.to_vec(),
            timestamp,
        }
    }

    /// Verify the embedded signature against `pubkey`.
    pub fn verify(&self) -> bool {
        let Ok(pk) = VerifyingKey::from_bytes(&self.pubkey) else {
            return false;
        };
        let sig_bytes: [u8; 64] = match self.signature.as_slice().try_into() {
            Ok(b) => b,
            Err(_) => return false,
        };
        let sig = Signature::from_bytes(&sig_bytes);
        pk.verify_strict(&sign_payload(&self.key, &self.value, self.timestamp), &sig)
            .is_ok()
    }
}

/// Logical wire messages exchanged over the transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Ping { request_id: u64, id: NodeId },
    Pong { request_id: u64, id: NodeId },
    FindNode { request_id: u64, id: NodeId, target: NodeId },
    FoundNodes { request_id: u64, id: NodeId, nodes: Vec<Contact> },
    Store { request_id: u64, id: NodeId, key: NodeId, value: StoredValue },
    StoreValue { request_id: u64, id: NodeId, key: NodeId, value: StoredValue },
}

fn msg_request_id(msg: &Message) -> u64 {
    match msg {
        Message::Ping { request_id, .. }
        | Message::Pong { request_id, .. }
        | Message::FindNode { request_id, .. }
        | Message::FoundNodes { request_id, .. }
        | Message::Store { request_id, .. }
        | Message::StoreValue { request_id, .. } => *request_id,
    }
}

/// Datagram transport abstraction (UDP in practice).
pub trait Transport: Send + Sync {
    fn local_addr(&self) -> SocketAddr;
    async fn send(&self, to: SocketAddr, msg: &Message) -> Result<()>;
    async fn recv(&self) -> Result<(SocketAddr, Message)>;
}

/// UDP implementation of [`Transport`] using bincode framing.
pub struct UdpTransport {
    socket: Arc<tokio::net::UdpSocket>,
}

impl UdpTransport {
    pub async fn new() -> anyhow::Result<Self> {
        let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
        Ok(Self {
            socket: Arc::new(socket),
        })
    }
}

impl Transport for UdpTransport {
    fn local_addr(&self) -> SocketAddr {
        self.socket.local_addr().unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap())
    }

    async fn send(&self, to: SocketAddr, msg: &Message) -> Result<()> {
        let data = bincode::serialize(msg).context("bincode serialize dht message")?;
        self.socket.send_to(&data, to).await?;
        Ok(())
    }

    async fn recv(&self) -> Result<(SocketAddr, Message)> {
        let mut buf = [0u8; 65536];
        let (n, from) = self.socket.recv_from(&mut buf).await?;
        let msg = bincode::deserialize(&buf[..n]).context("bincode deserialize dht message")?;
        Ok((from, msg))
    }
}

/// Kademlia routing table built from k-buckets.
pub(crate) struct RoutingTable {
    buckets: Vec<Vec<Contact>>,
}

impl RoutingTable {
    fn new() -> Self {
        Self {
            buckets: vec![Vec::new(); ID_BITS],
        }
    }

    fn clear(&mut self) {
        for b in &mut self.buckets {
            b.clear();
        }
    }

    /// Bucket holding nodes that differ from `local` at the given bit position.
    fn bucket_index(&self, local: &NodeId, other: &NodeId) -> usize {
        for i in 0..32 {
            let diff = local.0[i] ^ other.0[i];
            if diff != 0 {
                let leading = diff.leading_zeros() as usize;
                return ID_BITS - 1 - (i * 8 + leading);
            }
        }
        ID_BITS - 1
    }

    fn insert(&mut self, local: &NodeId, contact: Contact) {
        let bi = self.bucket_index(local, &contact.id);
        let bucket = &mut self.buckets[bi];
        if let Some(idx) = bucket.iter().position(|c| c.id == contact.id) {
            bucket.remove(idx);
            bucket.push(contact);
            return;
        }
        if bucket.len() < K {
            bucket.push(contact);
        } else {
            // Evict the least-recently-seen contact (front of the bucket).
            bucket.remove(0);
            bucket.push(contact);
        }
    }

    /// The up-to-`k` contacts closest to `target`, sorted by distance.
    fn closest(&self, target: &NodeId, k: usize) -> Vec<Contact> {
        let mut all: Vec<Contact> = self.buckets.iter().flatten().cloned().collect();
        all.sort_by(|a, b| a.id.distance(target).cmp(&b.id.distance(target)));
        all.truncate(k);
        all
    }
}

pub(crate) struct NodeInner {
    id: RwLock<NodeId>,
    transport: Arc<dyn Transport>,
    table: Mutex<RoutingTable>,
    store: Mutex<HashMap<NodeId, Vec<StoredValue>>>,
    pending: Mutex<HashMap<u64, mpsc::Sender<(SocketAddr, Message)>>>,
    next_request_id: AtomicU64,
    bootstrap: Mutex<Vec<SocketAddr>>,
}

impl NodeInner {
    fn current_id(&self) -> NodeId {
        *self.id.read().unwrap()
    }

    fn learn(&self, contact: Contact) {
        let mut table = self.table.lock().unwrap();
        table.insert(&self.current_id(), contact);
    }

    fn closest(&self, target: &NodeId) -> Vec<Contact> {
        let table = self.table.lock().unwrap();
        table.closest(target, K)
    }

    fn put_value(&self, key: NodeId, value: StoredValue) {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(key).or_default();
        if !entry.iter().any(|v| v.value == value.value) {
            entry.push(value);
        }
    }

    fn first_value(&self, key: &NodeId) -> Option<StoredValue> {
        let store = self.store.lock().unwrap();
        store.get(key).and_then(|v| v.first()).cloned()
    }

    fn fresh_values(&self, key: &NodeId) -> Vec<StoredValue> {
        let now = chrono::Utc::now().timestamp();
        let store = self.store.lock().unwrap();
        store
            .get(key)
            .map(|vs| {
                vs.iter()
                    .filter(|v| now - v.timestamp <= VALUE_TTL.as_secs() as i64)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn handle_message(&self, from: SocketAddr, msg: Message) -> Result<()> {
        match msg {
            Message::Ping { request_id, id } => {
                self.learn(Contact { id, addr: from });
                self.transport
                    .send(from, Message::Pong { request_id, id: self.current_id() })
                    .await?;
            }
            Message::Pong { id, .. } => {
                self.learn(Contact { id, addr: from });
            }
            Message::FindNode {
                request_id,
                id,
                target,
            } => {
                self.learn(Contact { id, addr: from });
                let nodes: Vec<Contact> = self
                    .closest(&target)
                    .into_iter()
                    .filter(|c| c.id != self.current_id())
                    .collect();
                self.transport
                    .send(
                        from,
                        Message::FoundNodes {
                            request_id,
                            id: self.current_id(),
                            nodes,
                        },
                    )
                    .await?;
                if let Some(value) = self.first_value(&target) {
                    self.transport
                        .send(
                            from,
                            Message::StoreValue {
                                request_id,
                                id: self.current_id(),
                                key: target,
                                value,
                            },
                        )
                        .await?;
                }
            }
            Message::FoundNodes { nodes, .. } => {
                for n in nodes {
                    self.learn(n);
                }
            }
            Message::Store {
                request_id,
                id,
                key,
                value,
            } => {
                self.learn(Contact { id, addr: from });
                self.put_value(key, value.clone());
                self.transport
                    .send(
                        from,
                        Message::StoreValue {
                            request_id,
                            id: self.current_id(),
                            key,
                            value,
                        },
                    )
                    .await?;
            }
            Message::StoreValue { id, key, value, .. } => {
                self.learn(Contact { id, addr: from });
                self.put_value(key, value);
            }
        }
        Ok(())
    }

    async fn query_collect(&self, addr: SocketAddr, msg: Message) -> Vec<Message> {
        let rid = msg_request_id(&msg);
        let (tx, mut rx) = mpsc::channel(16);
        self.pending.lock().unwrap().insert(rid, tx);
        if self.transport.send(addr, msg).await.is_err() {
            self.pending.lock().unwrap().remove(&rid);
            return Vec::new();
        }
        let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
        let mut replies = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some((_, reply))) => replies.push(reply),
                _ => break,
            }
        }
        self.pending.lock().unwrap().remove(&rid);
        replies
    }

    async fn query_ping(&self, addr: SocketAddr) -> Option<NodeId> {
        let rid = self.next_request_id.fetch_add(1, AtomicOrdering::SeqCst);
        let replies = self
            .query_collect(
                addr,
                Message::Ping {
                    request_id: rid,
                    id: self.current_id(),
                },
            )
            .await;
        replies
            .into_iter()
            .find_map(|m| match m {
                Message::Pong { id, .. } => Some(id),
                _ => None,
            })
    }

    async fn query_find_node(
        &self,
        addr: SocketAddr,
        target: &NodeId,
    ) -> (Vec<Contact>, Vec<StoredValue>) {
        let rid = self.next_request_id.fetch_add(1, AtomicOrdering::SeqCst);
        let replies = self
            .query_collect(
                addr,
                Message::FindNode {
                    request_id: rid,
                    id: self.current_id(),
                    target: *target,
                },
            )
            .await;
        let mut nodes = Vec::new();
        let mut values = Vec::new();
        for m in replies {
            match m {
                Message::FoundNodes { nodes: nn, .. } => nodes.extend(nn),
                Message::StoreValue { value, .. } => values.push(value),
                _ => {}
            }
        }
        (nodes, values)
    }

    async fn query_store(&self, addr: SocketAddr, key: NodeId, value: StoredValue) {
        let rid = self.next_request_id.fetch_add(1, AtomicOrdering::SeqCst);
        let _ = self
            .query_collect(
                addr,
                Message::Store {
                    request_id: rid,
                    id: self.current_id(),
                    key,
                    value,
                },
            )
            .await;
    }
}

/// A running Kademlia node. Cheap to clone (all state is shared).
#[derive(Clone)]
pub struct Node {
    inner: Arc<NodeInner>,
}

impl Node {
    pub fn new(id: NodeId, transport: Arc<dyn Transport>, bootstrap: Vec<SocketAddr>) -> Self {
        Self {
            inner: Arc::new(NodeInner {
                id: RwLock::new(id),
                transport,
                table: Mutex::new(RoutingTable::new()),
                store: Mutex::new(HashMap::new()),
                pending: Mutex::new(HashMap::new()),
                next_request_id: AtomicU64::new(1),
                bootstrap: Mutex::new(bootstrap),
            }),
        }
    }

    pub fn current_id(&self) -> NodeId {
        self.inner.current_id()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.inner.transport.local_addr()
    }

    /// Re-key the node to a stable identity id and forget stale routing state.
    pub fn set_id(&self, id: NodeId) {
        *self.inner.id.write().unwrap() = id;
        self.inner.table.lock().unwrap().clear();
    }

    /// Bootstrap from a single known peer address (for truly serverless operation).
    pub async fn bootstrap_from(&self, peer_addr: SocketAddr) -> Result<()> {
        if let Some(id) = self.inner.query_ping(peer_addr).await {
            self.inner.learn(Contact { id, addr: peer_addr });
            debug!(%peer_addr, "bootstrapped from peer");
        } else {
            debug!(%peer_addr, "peer unresponsive during bootstrap");
        }
        let _ = self.find_node(&self.current_id()).await?;
        Ok(())
    }

    /// Contact known bootstrap nodes and populate the routing table.
    pub async fn bootstrap(&self) -> Result<()> {
        let bootstrap_nodes = self.inner.bootstrap.lock().unwrap().clone();
        for addr in &bootstrap_nodes {
            if let Some(id) = self.inner.query_ping(*addr).await {
                self.inner.learn(Contact { id, addr: *addr });
                debug!(%addr, "bootstrapped against dht node");
            } else {
                debug!(%addr, "bootstrap node unresponsive");
            }
        }
        let _ = self.find_node(&self.current_id()).await?;
        Ok(())
    }

    /// Iterative nearest-neighbour lookup for `target`.
    pub async fn find_node(&self, target: &NodeId) -> Result<Vec<Contact>> {
        let mut seen: HashSet<NodeId> = HashSet::new();
        let mut closest = self.inner.closest(target);
        if closest.is_empty() {
            return Ok(closest);
        }
        for _ in 0..MAX_ROUNDS {
            let candidates: Vec<Contact> = closest
                .iter()
                .filter(|c| !seen.contains(&c.id))
                .cloned()
                .take(ALPHA)
                .collect();
            if candidates.is_empty() {
                break;
            }
            seen.extend(candidates.iter().map(|c| c.id));
            let mut merged = closest.clone();
            for c in &candidates {
                let (nodes, _values) = self.inner.query_find_node(c.addr, target).await;
                for n in nodes {
                    if n.id != self.current_id() && !merged.iter().any(|m| m.id == n.id) {
                        merged.push(n.clone());
                        self.inner.learn(n);
                    }
                }
            }
            merged.sort_by(|a, b| a.id.distance(target).cmp(&b.id.distance(target)));
            merged.truncate(K);
            let improved = merged != closest;
            closest = merged;
            if !improved {
                break;
            }
        }
        Ok(closest)
    }

    /// Store a value under `key`, replicating it to the closest nodes.
    pub async fn put(&self, key: &NodeId, value: StoredValue) -> Result<()> {
        self.inner.put_value(*key, value.clone());
        let nodes = self.find_node(key).await?;
        for n in nodes {
            self.inner.query_store(n.addr, *key, value.clone()).await;
        }
        Ok(())
    }

    /// Retrieve fresh, valid values stored under `key`.
    pub async fn get(&self, key: &NodeId) -> Result<Vec<StoredValue>> {
        let mut found = self.inner.fresh_values(key);
        let mut seen: HashSet<NodeId> = HashSet::new();
        let mut closest = self.inner.closest(key);
        if closest.is_empty() {
            return Ok(found);
        }
        for _ in 0..MAX_ROUNDS {
            let candidates: Vec<Contact> = closest
                .iter()
                .filter(|c| !seen.contains(&c.id))
                .cloned()
                .take(ALPHA)
                .collect();
            if candidates.is_empty() {
                break;
            }
            seen.extend(candidates.iter().map(|c| c.id));
            let mut merged = closest.clone();
            for c in &candidates {
                let (nodes, values) = self.inner.query_find_node(c.addr, key).await;
                for n in nodes {
                    if n.id != self.current_id() && !merged.iter().any(|m| m.id == n.id) {
                        merged.push(n.clone());
                        self.inner.learn(n);
                    }
                }
                for v in values {
                    if v.verify() {
                        if !found.iter().any(|f| f.value == v.value) {
                            found.push(v);
                        }
                    }
                }
            }
            if !found.is_empty() {
                break;
            }
            merged.sort_by(|a, b| a.id.distance(key).cmp(&b.id.distance(key)));
            merged.truncate(K);
            let improved = merged != closest;
            closest = merged;
            if !improved {
                break;
            }
        }
        Ok(found)
    }

    /// Run the receive loop. Spawn with `tokio::spawn(node.clone().run())`.
    pub async fn run(self: Arc<Self>) {
        let inner = self.inner.clone();
        run_loop(inner).await;
    }
}

async fn run_loop(inner: Arc<NodeInner>) {
    loop {
        match inner.transport.recv().await {
            Ok((from, msg)) => {
                let request_id = msg_request_id(&msg);
                let delivered = {
                    let pending = inner.pending.lock().unwrap();
                    match pending.get(&request_id) {
                        Some(tx) => tx.try_send((from, msg.clone())).is_ok(),
                        None => false,
                    }
                };
                if delivered {
                    continue;
                }
                if let Err(e) = inner.handle_message(from, msg).await {
                    warn!("failed handling dht message: {e}");
                }
            }
            Err(e) => {
                warn!("dht recv error: {e}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::keys::Ed25519KeyPair;
    use tokio::net::UdpSocket;

    async fn make_node(bootstrap: Vec<SocketAddr>, key: &Ed25519KeyPair) -> Arc<Node> {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let transport: Arc<dyn Transport> = Arc::new(UdpTransport {
            socket: Arc::new(sock),
        });
        let id = NodeId::from_pubkey(&key.public_key_bytes());
        let node = Arc::new(Node::new(id, transport, bootstrap));
        let runner = node.clone();
        tokio::spawn(async move { runner.run().await });
        node
    }

    fn two_nodes(_a_key: &Ed25519KeyPair, _b_key: &Ed25519KeyPair) {}

    #[tokio::test]
    async fn lookup_finds_node() {
        let a_key = Ed25519KeyPair::generate().unwrap();
        let b_key = Ed25519KeyPair::generate().unwrap();

        let b = make_node(vec![], &b_key).await;
        let a = make_node(vec![b.local_addr()], &a_key).await;

        a.bootstrap().await.unwrap();
        b.bootstrap().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let found = a.find_node(&b.current_id()).await.unwrap();
        assert!(
            found.iter().any(|c| c.id == b.current_id()),
            "lookup should discover node b, got: {found:?}"
        );
    }

    #[tokio::test]
    async fn put_get_round_trip() {
        let a_key = Ed25519KeyPair::generate().unwrap();
        let b_key = Ed25519KeyPair::generate().unwrap();

        let b = make_node(vec![], &b_key).await;
        let a = make_node(vec![b.local_addr()], &a_key).await;

        a.bootstrap().await.unwrap();
        b.bootstrap().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let key = NodeId::random();
        let value = StoredValue::new(key, b"hello".to_vec(), &a_key);
        assert!(value.verify());

        a.put(&key, value.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;

        let mut tampered = value.clone();
        tampered.value = b"tampered".to_vec();
        assert!(!tampered.verify());

        let got = b.get(&key).await.unwrap();
        assert!(
            got.iter().any(|v| v.value == b"hello".to_vec()),
            "peer should retrieve the replicated value, got: {got:?}"
        );
    }

    #[tokio::test]
    async fn self_healing_adds_new_node() {
        let a_key = Ed25519KeyPair::generate().unwrap();
        let b_key = Ed25519KeyPair::generate().unwrap();
        let c_key = Ed25519KeyPair::generate().unwrap();

        let b = make_node(vec![], &b_key).await;
        let a = make_node(vec![b.local_addr()], &a_key).await;

        a.bootstrap().await.unwrap();
        b.bootstrap().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        // A third node joins, bootstrapping through b.
        let c = make_node(vec![b.local_addr()], &c_key).await;
        c.bootstrap().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;

        // a must now discover c without being told about it directly.
        let found = a.find_node(&c.current_id()).await.unwrap();
        assert!(
            found.iter().any(|n| n.id == c.current_id()),
            "a should self-heal and discover c, got: {found:?}"
        );
    }
}
