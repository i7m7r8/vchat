use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMessage {
    pub version: u8,
    pub msg_type: WireMessageType,
    pub payload: Vec<u8>,
    pub timestamp: i64,
    pub sequence: u64,
    pub sender_pubkey: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WireMessageType {
    NoiseHandshake,
    NoisePayload,
    TextMessage,
    FileChunk,
    FileMeta,
    VoiceFrame,
    VideoFrame,
    ScreenFrame,
    KeyExchange,
    Heartbeat,
    Ack,
    CallInvite,
    CallAccept,
    CallReject,
    CallEnd,
    IceCandidate,
}

pub const WIRE_VERSION: u8 = 1;
pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024;
pub const MAX_FRAME_SIZE: usize = 65536;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPayload {
    pub content: String,
    pub reply_to: Option<String>,
    pub ephemeral_ttl: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetaPayload {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub encryption_key: Vec<u8>,
    pub checksum: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunkPayload {
    pub file_id: String,
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInvitePayload {
    pub call_id: String,
    pub call_type: CallType,
    pub sdp_offer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallType {
    Audio,
    Video,
    ScreenShare,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    pub uptime_secs: u64,
    pub active_sessions: u32,
}

pub fn create_wire_message(
    msg_type: WireMessageType,
    payload: Vec<u8>,
    sender_pubkey: &[u8],
    signing_key: &ed25519_dalek::SigningKey,
    sequence: u64,
) -> Result<WireMessage> {
    let timestamp = chrono::Utc::now().timestamp_millis() as i64;

    let mut sign_data = Vec::new();
    sign_data.push(WIRE_VERSION);
    sign_data.extend_from_slice(&(msg_type.clone() as u8).to_le_bytes());
    sign_data.extend_from_slice(&payload);
    sign_data.extend_from_slice(&timestamp.to_le_bytes());
    sign_data.extend_from_slice(&sequence.to_le_bytes());
    sign_data.extend_from_slice(sender_pubkey);

    let signature = signing_key.sign(&sign_data).to_bytes().to_vec();

    Ok(WireMessage {
        version: WIRE_VERSION,
        msg_type,
        payload,
        timestamp,
        sequence,
        sender_pubkey: sender_pubkey.to_vec(),
        signature,
    })
}

pub fn verify_wire_message(msg: &WireMessage, sender_pubkey: &[u8; 32]) -> Result<bool> {
    use ed25519_dalek::{Signature, VerifyingKey};

    if msg.version != WIRE_VERSION {
        return Ok(false);
    }

    let verifying = VerifyingKey::from_bytes(sender_pubkey)
        .map_err(|e| anyhow::anyhow!("Invalid public key: {e}"))?;

    let mut sign_data = Vec::new();
    sign_data.push(msg.version);
    sign_data.extend_from_slice(&(msg.msg_type.clone() as u8).to_le_bytes());
    sign_data.extend_from_slice(&msg.payload);
    sign_data.extend_from_slice(&msg.timestamp.to_le_bytes());
    sign_data.extend_from_slice(&msg.sequence.to_le_bytes());
    sign_data.extend_from_slice(&msg.sender_pubkey);

    let sig_bytes: [u8; 64] = msg
        .signature
        .clone()
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid signature length"))?;

    let signature = Signature::from_bytes(&sig_bytes);

    Ok(verifying.verify_strict(&sign_data, &signature).is_ok())
}

pub fn serialize_wire_message(msg: &WireMessage) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let json = serde_json::to_vec(msg)?;
    let len = json.len() as u32;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&json);
    Ok(buf)
}

pub fn deserialize_wire_message(data: &[u8]) -> Result<WireMessage> {
    if data.len() < 4 {
        anyhow::bail!("Message too short");
    }
    let len = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
    if data.len() < 4 + len {
        anyhow::bail!("Incomplete message");
    }
    if len > MAX_MESSAGE_SIZE {
        anyhow::bail!("Message too large: {len} bytes");
    }
    serde_json::from_slice(&data[4..4 + len])
        .map_err(|e| anyhow::anyhow!("Deserialization failed: {e}"))
}
