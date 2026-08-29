use anyhow::{Context, Result};
use stun::client::StunClient;
use stun::message::Message;
use stun::attributes::{MappedAddress, XorMappedAddress, Username, Realm, Nonce};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket as TokioUdpSocket;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceCandidate {
    pub foundation: String,
    pub component_id: u16,
    pub protocol: String, // "udp" or "tcp"
    pub priority: u32,
    pub address: IpAddr,
    pub port: u16,
    pub candidate_type: String, // "host", "srflx", "relay"
    pub related_address: Option<IpAddr>,
    pub related_port: Option<u16>,
}

impl IceCandidate {
    pub fn to_sdp(&self) -> String {
        let typ = match self.candidate_type.as_str() {
            "host" => "host",
            "srflx" => "srflx",
            "relay" => "relay",
            _ => "host",
        };

        let raddr = self.related_address.map(|a| format!(" raddr {}", a)).unwrap_or_default();
        let rport = self.related_port.map(|p| format!(" rport {}", p)).unwrap_or_default();

        format!(
            "a=candidate:{} {} {} {} {} {} typ {}{}{}",
            self.foundation,
            self.component_id,
            self.protocol,
            self.priority,
            self.address,
            self.port,
            typ,
            raddr,
            rport
        )
    }

    pub fn to_socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.address, self.port)
    }
}

#[derive(Debug, Clone)]
pub struct IceConfig {
    pub stun_servers: Vec<String>,
    pub turn_servers: Vec<TurnServer>,
    pub ice_lite: bool,
}

#[derive(Debug, Clone)]
pub struct TurnServer {
    pub url: String,
    pub username: String,
    pub credential: String,
}

impl Default for IceConfig {
    fn default() -> Self {
        Self {
            stun_servers: vec![
                "stun:stun.l.google.com:19302".to_string(),
                "stun:stun1.l.google.com:19302".to_string(),
                "stun:stun2.l.google.com:19302".to_string(),
            ],
            turn_servers: vec![],
            ice_lite: false,
        }
    }
}

pub struct IceAgent {
    config: IceConfig,
    local_candidates: Arc<RwLock<Vec<IceCandidate>>>,
    remote_candidates: Arc<RwLock<Vec<IceCandidate>>>,
    selected_pair: Arc<RwLock<Option<(IceCandidate, IceCandidate)>>>,
    controlling: bool,
    tie_breaker: u64,
}

impl IceAgent {
    pub fn new(config: IceConfig, controlling: bool) -> Self {
        Self {
            config,
            local_candidates: Arc::new(RwLock::new(Vec::new())),
            remote_candidates: Arc::new(RwLock::new(Vec::new())),
            selected_pair: Arc::new(RwLock::new(None)),
            controlling,
            tie_breaker: rand::random(),
        }
    }

    pub async fn gather_candidates(&self) -> Result<Vec<IceCandidate>> {
        let mut candidates = Vec::new();

        // 1. Host candidates (local interfaces)
        let host_candidates = self.gather_host_candidates().await?;
        candidates.extend(host_candidates);

        // 2. Server reflexive candidates (STUN)
        if !self.config.ice_lite {
            let srflx_candidates = self.gather_srflx_candidates().await?;
            candidates.extend(srflx_candidates);
        }

        // 3. Relay candidates (TURN)
        if !self.config.turn_servers.is_empty() && !self.config.ice_lite {
            let relay_candidates = self.gather_relay_candidates().await?;
            candidates.extend(relay_candidates);
        }

        let mut local = self.local_candidates.write().await;
        *local = candidates.clone();

        Ok(candidates)
    }

    async fn gather_host_candidates(&self) -> Result<Vec<IceCandidate>> {
        let mut candidates = Vec::new();
        let interfaces = local_ip_address::list_afinet_netifas()
            .context("Failed to list network interfaces")?;

        for (idx, iface) in interfaces.iter().enumerate() {
            let foundation = format!("host-{}", idx);
            let candidate = IceCandidate {
                foundation,
                component_id: 1,
                protocol: "udp".to_string(),
                priority: 2_130_706_431, // (2^24) * 126 + 1
                address: iface.addr,
                port: 0, // Will be set when socket is bound
                candidate_type: "host".to_string(),
                related_address: None,
                related_port: None,
            };
            candidates.push(candidate);
        }

        Ok(candidates)
    }

    async fn gather_srflx_candidates(&self) -> Result<Vec<IceCandidate>> {
        let mut candidates = Vec::new();

        for stun_url in &self.config.stun_servers {
            if let Ok(cands) = self.query_stun(stun_url).await {
                candidates.extend(cands);
            }
        }

        Ok(candidates)
    }

    async fn query_stun(&self, stun_url: &str) -> Result<Vec<IceCandidate>> {
        let url = stun_url.strip_prefix("stun:").unwrap_or(stun_url);
        let addr: SocketAddr = url.parse().context("Invalid STUN URL")?;

        let socket = TokioUdpSocket::bind("0.0.0.0:0").await?;
        let mut client = StunClient::new(socket);

        let mut msg = Message::new(stun::message::MessageType::BindingRequest);
        msg.add_attribute(stun::attributes::Attribute::Fingerprint);

        let response = client
            .send_receive(&msg, addr)
            .await
            .context("STUN request failed")?;

        let mut candidates = Vec::new();

        if let Some(xor_mapped) = response.get_attribute::<XorMappedAddress>() {
            let mapped_addr = xor_mapped.to_socket_addr();
            let candidate = IceCandidate {
                foundation: format!("srflx-{}", rand::random::<u32>()),
                component_id: 1,
                protocol: "udp".to_string(),
                priority: 1_694_498_815, // (2^24) * 100 + 1
                address: mapped_addr.ip(),
                port: mapped_addr.port(),
                candidate_type: "srflx".to_string(),
                related_address: None,
                related_port: None,
            };
            candidates.push(candidate);
        }

        Ok(candidates)
    }

    async fn gather_relay_candidates(&self) -> Result<Vec<IceCandidate>> {
        let mut candidates = Vec::new();

        for turn in &self.config.turn_servers {
            // TURN allocation logic would go here
            // This requires TURN client implementation
            debug!("TURN server configured: {}", turn.url);
        }

        Ok(candidates)
    }

    pub async fn add_remote_candidates(&self, candidates: Vec<IceCandidate>) {
        let mut remote = self.remote_candidates.write().await;
        *remote = candidates;
    }

    pub async fn start_connectivity_checks(&self) -> Result<()> {
        let local = self.local_candidates.read().await.clone();
        let remote = self.remote_candidates.read().await.clone();

        if local.is_empty() || remote.is_empty() {
            return Err(anyhow::anyhow!("No candidates available"));
        }

        // Sort by priority (highest first)
        let mut pairs = Vec::new();
        for l in &local {
            for r in &remote {
                if l.protocol == r.protocol {
                    let priority = self.compute_pair_priority(l, r);
                    pairs.push((priority, l.clone(), r.clone()));
                }
            }
        }

        pairs.sort_by(|a, b| b.0.cmp(&a.0));

        // Perform connectivity checks
        for (_, local_cand, remote_cand) in pairs {
            if self.check_connectivity(&local_cand, &remote_cand).await? {
                let mut selected = self.selected_pair.write().await;
                *selected = Some((local_cand, remote_cand));
                info!("ICE connectivity established: {} <-> {}",
                    local_cand.to_socket_addr(), remote_cand.to_socket_addr());
                return Ok(());
            }
        }

        Err(anyhow::anyhow!("No working candidate pair found"))
    }

    fn compute_pair_priority(&self, local: &IceCandidate, remote: &IceCandidate) -> u64 {
        let g = if self.controlling { local.priority } else { remote.priority };
        let d = if self.controlling { remote.priority } else { local.priority };
        ((g as u64) << 32) | (d as u64) | ((self.tie_breaker as u64) << 1)
    }

    async fn check_connectivity(&self, local: &IceCandidate, remote: &IceCandidate) -> Result<bool> {
        // Send STUN binding request to remote candidate
        let socket = TokioUdpSocket::bind("0.0.0.0:0").await?;
        socket.connect(remote.to_socket_addr()).await?;

        let mut msg = Message::new(stun::message::MessageType::BindingRequest);
        msg.add_attribute(stun::attributes::Attribute::Fingerprint);

        let mut client = StunClient::new(socket);
        match tokio::time::timeout(Duration::from_secs(3), client.send_receive(&msg, remote.to_socket_addr())).await {
            Ok(Ok(_)) => {
                debug!("Connectivity check succeeded: {} -> {}", local.to_socket_addr(), remote.to_socket_addr());
                Ok(true)
            }
            _ => {
                debug!("Connectivity check failed: {} -> {}", local.to_socket_addr(), remote.to_socket_addr());
                Ok(false)
            }
        }
    }

    pub async fn get_selected_pair(&self) -> Option<(IceCandidate, IceCandidate)> {
        self.selected_pair.read().await.clone()
    }
}

pub async fn create_ice_agent(config: IceConfig, controlling: bool) -> Result<IceAgent> {
    let agent = IceAgent::new(config, controlling);
    agent.gather_candidates().await?;
    Ok(agent)
}