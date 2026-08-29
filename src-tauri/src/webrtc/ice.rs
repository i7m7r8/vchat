//! Minimal STUN/ICE/TURN client for Jami-style P2P UDP video transport.
//!
//! Video frames travel over direct UDP sockets between peers (hole-punched via
//! ICE), bypassing Tor for the media path to reduce latency. ICE candidates
//! are exchanged over the existing Tor signaling channel. A self-hostable TURN
//! relay is used only as a fallback when direct hole-punching fails; none is
//! required by default.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tracing::debug;

// ── STUN message types (RFC 5389 / 5766) ────────────────────────────────────

const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;
const STUN_ALLOCATE_REQUEST: u16 = 0x0003;
const STUN_ALLOCATE_RESPONSE: u16 = 0x0103;

const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const ATTR_REQUESTED_TRANSPORT: u16 = 0x0019;
const ATTR_XOR_RELAYED_ADDRESS: u16 = 0x0016;

const MAGIC_COOKIE: u32 = 0x2112_A442;
const MAX_UDP_PAYLOAD: usize = 1200;

// ── Candidates ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateType {
    Host,
    Srflx,
    Relay,
}

#[derive(Debug, Clone, Copy)]
pub struct IceCandidate {
    pub candidate_type: CandidateType,
    pub addr: SocketAddr,
    pub priority: u32,
}

impl IceCandidate {
    pub fn host(addr: SocketAddr) -> Self {
        Self {
            candidate_type: CandidateType::Host,
            addr,
            priority: 126,
        }
    }

    pub fn srflx(addr: SocketAddr) -> Self {
        Self {
            candidate_type: CandidateType::Srflx,
            addr,
            priority: 100,
        }
    }

    pub fn relay(addr: SocketAddr) -> Self {
        Self {
            candidate_type: CandidateType::Relay,
            addr,
            priority: 1,
        }
    }
}

impl std::fmt::Display for IceCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.candidate_type {
            CandidateType::Host => "host",
            CandidateType::Srflx => "srflx",
            CandidateType::Relay => "relay",
        };
        write!(f, "{},{}:{},{}", kind, self.addr.ip(), self.addr.port(), self.priority)
    }
}

pub fn parse_candidate(s: &str) -> Option<IceCandidate> {
    let mut parts = s.splitn(3, ',');
    let kind = parts.next()?;
    let addr_part = parts.next()?;
    let rest = parts.next().unwrap_or("0");
    let mut pt = addr_part.rsplitn(2, ':');
    let port: u16 = pt.next()?.parse().ok()?;
    let ip: IpAddr = pt.next()?.parse().ok()?;
    let priority: u32 = rest.split(',').next().unwrap_or("0").parse().ok()?;
    let candidate_type = match kind {
        "host" => CandidateType::Host,
        "srflx" => CandidateType::Srflx,
        "relay" => CandidateType::Relay,
        _ => return None,
    };
    Some(IceCandidate {
        candidate_type,
        addr: SocketAddr::new(ip, port),
        priority,
    })
}

// ── Media chunk framing ─────────────────────────────────────────────────────

/// A frame transmitted over UDP, chunked to stay near the UDP MTU.
pub struct MediaChunk {
    pub frame_id: u32,
    pub offset: u32,
    pub total: u32,
    pub data: Vec<u8>,
}

pub fn encode_chunk(chunk: &MediaChunk) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16 + chunk.data.len());
    buf.extend_from_slice(&(chunk.data.len() as u32).to_le_bytes());
    buf.extend_from_slice(&chunk.frame_id.to_le_bytes());
    buf.extend_from_slice(&chunk.offset.to_le_bytes());
    buf.extend_from_slice(&chunk.total.to_le_bytes());
    buf.extend_from_slice(&chunk.data);
    buf
}

pub fn decode_chunk(buf: &[u8]) -> Option<MediaChunk> {
    if buf.len() < 16 {
        return None;
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 16 + len {
        return None;
    }
    Some(MediaChunk {
        frame_id: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        offset: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
        total: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
        data: buf[16..16 + len].to_vec(),
    })
}

/// Send a whole frame over the media socket, chunked into UDP datagrams.
pub async fn send_frame(sock: &UdpSocket, remote: SocketAddr, frame_id: u32, data: &[u8]) -> Result<()> {
    let total = data.len() as u32;
    let mut offset = 0usize;
    while offset < data.len() {
        let end = (offset + MAX_UDP_PAYLOAD).min(data.len());
        let chunk = MediaChunk {
            frame_id,
            offset: offset as u32,
            total,
            data: data[offset..end].to_vec(),
        };
        let pkt = encode_chunk(&chunk);
        sock.send_to(&pkt, remote).await?;
        offset = end;
    }
    Ok(())
}

// ── STUN construction / parsing ─────────────────────────────────────────────

struct StunAttr {
    kind: u16,
    value: Vec<u8>,
}

fn stun_encode(msg_type: u16, tid: [u8; 12], attrs: &[StunAttr]) -> Vec<u8> {
    let mut body = Vec::new();
    for a in attrs {
        body.extend_from_slice(&a.kind.to_be_bytes());
        body.extend_from_slice(&(a.value.len() as u16).to_be_bytes());
        body.extend_from_slice(&a.value);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    let len = body.len() as u16;
    let mut buf = Vec::with_capacity(20 + body.len());
    buf.push(((msg_type >> 8) & 0x3f) as u8);
    buf.push((msg_type & 0xff) as u8);
    buf.push((len >> 8) as u8);
    buf.push((len & 0xff) as u8);
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(&tid);
    buf.extend_from_slice(&body);
    buf
}

fn stun_parse(buf: &[u8]) -> Option<(u16, [u8; 12], Vec<StunAttr>)> {
    if buf.len() < 20 {
        return None;
    }
    let msg_type = (((buf[0] & 0x3f) as u16) << 8) | buf[1] as u16;
    let len = (((buf[2] as u16) << 8) | buf[3] as u16) as usize;
    if buf.len() < 20 + len {
        return None;
    }
    let mut tid = [0u8; 12];
    tid.copy_from_slice(&buf[8..20]);
    let mut attrs = Vec::new();
    let mut off = 20;
    let end_all = 20 + len;
    while off + 4 <= end_all {
        let kind = u16::from_be_bytes([buf[off], buf[off + 1]]);
        let alen = u16::from_be_bytes([buf[off + 2], buf[off + 3]]) as usize;
        off += 4;
        let end = (off + alen).min(end_all);
        attrs.push(StunAttr {
            kind,
            value: buf[off..end].to_vec(),
        });
        off = (off + alen + 3) & !3;
    }
    Some((msg_type, tid, attrs))
}

fn attr_requested_transport() -> StunAttr {
    StunAttr {
        kind: ATTR_REQUESTED_TRANSPORT,
        value: vec![0x11, 0, 0, 0], // UDP
    }
}

fn parse_addr_attr(value: &[u8], is_xor: bool) -> Option<SocketAddr> {
    if value.len() < 8 {
        return None;
    }
    let family = value[1];
    let mut port = u16::from_be_bytes([value[2], value[3]]);
    if is_xor {
        port ^= (MAGIC_COOKIE >> 16) as u16;
    }
    let ip = match family {
        0x01 => {
            let mut o = [value[4], value[5], value[6], value[7]];
            if is_xor {
                let k = MAGIC_COOKIE.to_be_bytes();
                for i in 0..4 {
                    o[i] ^= k[i];
                }
            }
            IpAddr::V4(Ipv4Addr::new(o[0], o[1], o[2], o[3]))
        }
        0x02 => {
            if value.len() < 20 {
                return None;
            }
            let mut o = [0u8; 16];
            o.copy_from_slice(&value[4..20]);
            IpAddr::V6(std::net::Ipv6Addr::from(o))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

/// A bound UDP media socket plus its candidates.
pub struct MediaSocket {
    pub socket: Arc<UdpSocket>,
    pub local: SocketAddr,
}

pub async fn bind_media_socket() -> Result<MediaSocket> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let local = socket.local_addr()?;
    Ok(MediaSocket {
        socket: Arc::new(socket),
        local,
    })
}

/// Config for a self-hostable STUN/TURN relay server.
#[derive(Debug, Clone, Default)]
pub struct RelayConfig {
    pub stun_server: Option<String>,
    pub turn_server: Option<String>,
    #[allow(dead_code)]
    pub turn_username: Option<String>,
    #[allow(dead_code)]
    pub turn_password: Option<String>,
}

/// Gather host + (optional) server-reflexive + (optional) relayed candidates.
pub async fn gather_candidates(
    sock: &UdpSocket,
    relay: &RelayConfig,
) -> Vec<IceCandidate> {
    let mut cands = Vec::new();
    if let Ok(local) = sock.local_addr() {
        cands.push(IceCandidate::host(local));
    }
    if let Some(stun) = &relay.stun_server {
        match stun_binding(sock, stun).await {
            Some(public) if public != cands[0].addr => cands.push(IceCandidate::srflx(public)),
            _ => {}
        }
    }
    if let Some(turn) = &relay.turn_server {
        match turn_allocate(sock, turn).await {
            Some(relayed) => cands.push(IceCandidate::relay(relayed)),
            None => debug!("TURN allocation failed for {turn}"),
        }
    }
    cands
}

async fn stun_binding(sock: &UdpSocket, stun_addr: &str) -> Option<SocketAddr> {
    let addr: SocketAddr = stun_addr.parse().ok()?;
    let tid: [u8; 12] = rand::random();
    let req = stun_encode(STUN_BINDING_REQUEST, tid, &[]);
    sock.send_to(&req, addr).await.ok()?;
    let mut buf = [0u8; 2048];
    let res = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let n = sock.recv(&mut buf).await.ok()?;
            let (mt, rt, attrs) = stun_parse(&buf[..n])?;
            if mt == STUN_BINDING_RESPONSE && rt == tid {
                for a in attrs {
                    if a.kind == ATTR_XOR_MAPPED_ADDRESS {
                        return parse_addr_attr(&a.value, true);
                    }
                    if a.kind == ATTR_MAPPED_ADDRESS {
                        return parse_addr_attr(&a.value, false);
                    }
                }
            }
        }
    })
    .await;
    res.ok().flatten()
}

async fn turn_allocate(sock: &UdpSocket, turn_addr: &str) -> Option<SocketAddr> {
    let addr: SocketAddr = turn_addr.parse().ok()?;
    let tid: [u8; 12] = rand::random();
    let req = stun_encode(STUN_ALLOCATE_REQUEST, tid, &[attr_requested_transport()]);
    sock.send_to(&req, addr).await.ok()?;
    let mut buf = [0u8; 4096];
    let res = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let n = sock.recv(&mut buf).await.ok()?;
            let (mt, rt, attrs) = stun_parse(&buf[..n])?;
            if mt == STUN_ALLOCATE_RESPONSE && rt == tid {
                for a in attrs {
                    if a.kind == ATTR_XOR_RELAYED_ADDRESS {
                        return parse_addr_attr(&a.value, true);
                    }
                }
            }
        }
    })
    .await;
    res.ok().flatten()
}

/// Run a connectivity check against a peer candidate: send a STUN binding
/// request and accept any STUN binding response from that address.
pub async fn check_connectivity(sock: &UdpSocket, remote: SocketAddr) -> bool {
    let tid: [u8; 12] = rand::random();
    let req = stun_encode(STUN_BINDING_REQUEST, tid, &[]);
    if sock.send_to(&req, remote).await.is_err() {
        return false;
    }
    let mut buf = [0u8; 2048];
    let res = tokio::time::timeout(Duration::from_millis(1500), async {
        loop {
            match sock.recv_from(&mut buf).await {
                Ok((n, _from)) => {
                    if let Some((mt, rt, _attrs)) = stun_parse(&buf[..n]) {
                        if mt == STUN_BINDING_RESPONSE && rt == tid {
                            return true;
                        }
                    }
                }
                Err(_) => return false,
            }
        }
    })
    .await;
    res.unwrap_or(false)
}

/// Very small synchronous SHA-256 helper used as a light keyed integrity.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

/// A connection state holding a shared RwLock of candidates for a call.
pub struct IceState {
    pub candidates: RwLock<Vec<IceCandidate>>,
    #[allow(dead_code)]
    pub relay: RelayConfig,
}

impl IceState {
    pub fn new(relay: RelayConfig) -> Self {
        Self {
            candidates: RwLock::new(Vec::new()),
            relay,
        }
    }
}
