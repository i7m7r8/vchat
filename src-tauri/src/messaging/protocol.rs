use anyhow::{bail, Context, Result};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const WIRE_VERSION: u8 = 1;
pub const MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;
pub const MAX_FRAME_SIZE: usize = 65_536;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WireMessageType {
    Text,
    FileMeta,
    FileChunk,
    CallInvite,
    CallAccept,
    CallReject,
    CallEnd,
    Heartbeat,
    Reaction,
    TypingIndicator,
    Presence,
    GroupCreate,
    GroupInvite,
    GroupMessage,
    GroupAck,
    DisappearingMessage,
    ReadReceipt,
    DeliveryReceipt,
    ProfileUpdate,
    GroupUpdate,
    Ack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMessage {
    pub version: u8,
    pub msg_type: WireMessageType,
    pub payload: Vec<u8>,
    pub signature: Vec<u8>,
    pub sender_pubkey: Vec<u8>,
    pub timestamp: i64,
    pub sequence: u64,
    pub nonce: Vec<u8>,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CallType {
    Voice,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextPayload {
    pub content: String,
    pub reply_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetaPayload {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub chunks_total: u32,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunkPayload {
    pub file_id: String,
    pub chunk_index: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInvitePayload {
    pub call_id: String,
    pub call_type: CallType,
    pub sdp_offer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    pub relay_hint: Option<String>,
    pub uptime_secs: u64,
    pub active_sessions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReactionPayload {
    pub message_id: String,
    pub emoji: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypingPayload {
    pub peer_onion: String,
    pub is_typing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresencePayload {
    pub status: String,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMemberInfo {
    pub onion_address: String,
    pub public_key: String,
    pub display_name: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupCreatePayload {
    pub group_id: String,
    pub name: String,
    pub members: Vec<GroupMemberInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMessagePayload {
    pub group_id: String,
    pub content: String,
    pub reply_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupAckPayload {
    pub group_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisappearingPayload {
    pub message_id: String,
    pub ttl_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadReceiptPayload {
    pub message_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReceiptPayload {
    pub message_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileUpdatePayload {
    pub display_name: Option<String>,
    pub avatar_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupUpdatePayload {
    pub group_id: String,
    pub update_type: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInvitePayload {
    pub group_id: String,
    pub group_name: String,
    pub invited_by: String,
    pub encrypted_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct GroupAckPayloadInner {
    pub group_id: String,
    pub message_id: String,
    pub status: String,
}

pub fn create_wire_message(
    signing_key: &SigningKey,
    verifying_key: &VerifyingKey,
    msg_type: WireMessageType,
    payload_bytes: Vec<u8>,
    message_id: String,
    sequence: u64,
) -> Result<WireMessage> {
    let timestamp = Utc::now().timestamp();
    let nonce: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();

    let mut signing_input = Vec::new();
    signing_input.extend_from_slice(&[WIRE_VERSION]);
    signing_input.extend_from_slice(&(msg_type.clone() as u16).to_le_bytes());
    signing_input.extend_from_slice(&(payload_bytes.len() as u32).to_le_bytes());
    signing_input.extend_from_slice(&payload_bytes);
    signing_input.extend_from_slice(&timestamp.to_le_bytes());
    signing_input.extend_from_slice(&sequence.to_le_bytes());
    signing_input.extend_from_slice(&nonce);
    signing_input.extend_from_slice(message_id.as_bytes());

    let signature = signing_key.sign(&signing_input);
    let signature_bytes = signature.to_bytes().to_vec();

    Ok(WireMessage {
        version: WIRE_VERSION,
        msg_type,
        payload: payload_bytes,
        signature: signature_bytes,
        sender_pubkey: verifying_key.to_bytes().to_vec(),
        timestamp,
        sequence,
        nonce,
        message_id,
    })
}

pub fn verify_wire_message(message: &WireMessage, verifying_key: &VerifyingKey) -> Result<bool> {
    if message.version != WIRE_VERSION {
        bail!(
            "Unsupported wire version: expected {}, got {}",
            WIRE_VERSION,
            message.version
        );
    }

    if message.payload.len() > MAX_MESSAGE_SIZE {
        bail!(
            "Payload size {} exceeds maximum {}",
            message.payload.len(),
            MAX_MESSAGE_SIZE
        );
    }

    let mut signing_input = Vec::new();
    signing_input.extend_from_slice(&[message.version]);
    signing_input.extend_from_slice(&(message.msg_type.clone() as u16).to_le_bytes());
    signing_input.extend_from_slice(&(message.payload.len() as u32).to_le_bytes());
    signing_input.extend_from_slice(&message.payload);
    signing_input.extend_from_slice(&message.timestamp.to_le_bytes());
    signing_input.extend_from_slice(&message.sequence.to_le_bytes());
    signing_input.extend_from_slice(&message.nonce);
    signing_input.extend_from_slice(message.message_id.as_bytes());

    if message.signature.len() != 64 {
        bail!(
            "Invalid signature length: expected 64, got {}",
            message.signature.len()
        );
    }

    let sig_bytes: [u8; 64] = message
        .signature
        .as_slice()
        .try_into()
        .context("Failed to convert signature to fixed array")?;

    let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);

    match verifying_key.verify(&signing_input, &signature) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

pub fn serialize_wire_message(message: &WireMessage) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let serialized = bincode::serialize(message).context("Failed to bincode-serialize WireMessage")?;

    buf.extend_from_slice(&(serialized.len() as u32).to_le_bytes());
    buf.extend_from_slice(&serialized);

    if buf.len() > MAX_FRAME_SIZE {
        bail!(
            "Serialized frame size {} exceeds MAX_FRAME_SIZE {}",
            buf.len(),
            MAX_FRAME_SIZE
        );
    }

    Ok(buf)
}

pub fn deserialize_wire_message(data: &[u8]) -> Result<WireMessage> {
    if data.len() < 4 {
        bail!("Data too short to contain frame length prefix");
    }

    let frame_len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;

    if data.len() < 4 + frame_len {
        bail!(
            "Insufficient data: expected {} bytes after header, got {}",
            frame_len,
            data.len() - 4
        );
    }

    if frame_len > MAX_FRAME_SIZE {
        bail!(
            "Frame length {} exceeds MAX_FRAME_SIZE {}",
            frame_len,
            MAX_FRAME_SIZE
        );
    }

    let payload = &data[4..4 + frame_len];
    let message: WireMessage =
        bincode::deserialize(payload).context("Failed to bincode-deserialize WireMessage")?;

    if message.version != WIRE_VERSION {
        bail!(
            "Deserialized message has unsupported version: {}",
            message.version
        );
    }

    Ok(message)
}

pub fn payload_to_json<T: Serialize>(payload: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(payload).context("Failed to serialize payload to JSON")
}

pub fn payload_from_json<T: for<'de> Deserialize<'de>>(data: &[u8]) -> Result<T> {
    serde_json::from_slice(data).context("Failed to deserialize payload from JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn make_test_message() -> (SigningKey, VerifyingKey, WireMessage) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let text = TextPayload {
            content: "hello world".to_string(),
            reply_to: None,
        };

        let msg = create_wire_message(
            &signing_key,
            &verifying_key,
            WireMessageType::Text,
            payload_to_json(&text).unwrap(),
            "test-msg-001".to_string(),
            0,
        )
        .unwrap();

        (signing_key, verifying_key, msg)
    }

    #[test]
    fn test_create_and_verify() {
        let (_, verifying_key, msg) = make_test_message();
        assert!(verify_wire_message(&msg, &verifying_key).unwrap());
    }

    #[test]
    fn test_verify_wrong_key() {
        let (_, _, msg) = make_test_message();
        let other_key = SigningKey::generate(&mut OsRng);
        assert!(!verify_wire_message(&msg, &other_key.verifying_key()).unwrap());
    }

    #[test]
    fn test_serialize_roundtrip() {
        let (_, verifying_key, msg) = make_test_message();
        let serialized = serialize_wire_message(&msg).unwrap();
        let deserialized = deserialize_wire_message(&serialized).unwrap();
        assert_eq!(msg.message_id, deserialized.message_id);
        assert_eq!(msg.msg_type, deserialized.msg_type);
        assert!(verify_wire_message(&deserialized, &verifying_key).unwrap());
    }

    #[test]
    fn test_text_payload_roundtrip() {
        let text = TextPayload {
            content: "test".to_string(),
            reply_to: Some("other-id".to_string()),
        };
        let bytes = payload_to_json(&text).unwrap();
        let decoded: TextPayload = payload_from_json(&bytes).unwrap();
        assert_eq!(text, decoded);
    }

    #[test]
    fn test_reaction_payload_roundtrip() {
        let reaction = ReactionPayload {
            message_id: "msg-123".to_string(),
            emoji: "🔥".to_string(),
        };
        let bytes = payload_to_json(&reaction).unwrap();
        let decoded: ReactionPayload = payload_from_json(&bytes).unwrap();
        assert_eq!(reaction, decoded);
    }

    #[test]
    fn test_group_create_payload_roundtrip() {
        let group = GroupCreatePayload {
            group_id: "grp-abc".to_string(),
            name: "Test Group".to_string(),
            members: vec![GroupMemberInfo {
                onion_address: "abc123.onion".to_string(),
                public_key: "pk123".to_string(),
                display_name: "Alice".to_string(),
                role: "admin".to_string(),
            }],
        };
        let bytes = payload_to_json(&group).unwrap();
        let decoded: GroupCreatePayload = payload_from_json(&bytes).unwrap();
        assert_eq!(group.group_id, decoded.group_id);
        assert_eq!(group.members.len(), decoded.members.len());
    }

    #[test]
    fn test_group_update_payload_roundtrip() {
        let update = GroupUpdatePayload {
            group_id: "grp-xyz".to_string(),
            update_type: "name_change".to_string(),
            data: serde_json::json!({"new_name": "Renamed Group"}),
        };
        let bytes = payload_to_json(&update).unwrap();
        let decoded: GroupUpdatePayload = payload_from_json(&bytes).unwrap();
        assert_eq!(update.group_id, decoded.group_id);
        assert_eq!(update.data, decoded.data);
    }

    #[test]
    fn test_disappearing_payload_roundtrip() {
        let dp = DisappearingPayload {
            message_id: "msg-456".to_string(),
            ttl_secs: 300,
        };
        let bytes = payload_to_json(&dp).unwrap();
        let decoded: DisappearingPayload = payload_from_json(&bytes).unwrap();
        assert_eq!(dp, decoded);
    }
}
