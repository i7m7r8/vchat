use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalMessage {
    pub message_type: SignalMessageType,
    pub payload: Vec<u8>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignalMessageType {
    Handshake,
    KeyExchange,
    Message,
    MediaControl,
    ScreenShareControl,
    Heartbeat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakePayload {
    pub public_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyExchangePayload {
    pub ephemeral_public: Vec<u8>,
    pub encrypted_key: Vec<u8>,
    pub mac: Vec<u8>,
}

pub fn create_handshake_message(
    public_key: &[u8],
    nonce: &[u8],
) -> Result<SignalMessage> {
    let payload = HandshakePayload {
        public_key: public_key.to_vec(),
        nonce: nonce.to_vec(),
        timestamp: chrono::Utc::now().timestamp(),
    };

    Ok(SignalMessage {
        message_type: SignalMessageType::Handshake,
        payload: serde_json::to_vec(&payload)?,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

pub fn create_key_exchange_message(
    ephemeral_public: &[u8],
    encrypted_key: &[u8],
    mac: &[u8],
) -> Result<SignalMessage> {
    let payload = KeyExchangePayload {
        ephemeral_public: ephemeral_public.to_vec(),
        encrypted_key: encrypted_key.to_vec(),
        mac: mac.to_vec(),
    };

    Ok(SignalMessage {
        message_type: SignalMessageType::KeyExchange,
        payload: serde_json::to_vec(&payload)?,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

pub fn serialize_signal_message(message: &SignalMessage) -> Result<Vec<u8>> {
    serde_json::to_vec(message).map_err(|e| e.into())
}

pub fn deserialize_signal_message(data: &[u8]) -> Result<SignalMessage> {
    serde_json::from_slice(data).map_err(|e| e.into())
}
