use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Serialize, Deserialize)]
pub enum VchatError {
    Crypto(String),
    Tor(String),
    Network(String),
    Storage(String),
    Protocol(String),
    Qr(String),
    WebRtc(String),
    Auth(String),
}

impl fmt::Display for VchatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Crypto(e) => write!(f, "CRYPTO: {e}"),
            Self::Tor(e) => write!(f, "TOR: {e}"),
            Self::Network(e) => write!(f, "NETWORK: {e}"),
            Self::Storage(e) => write!(f, "STORAGE: {e}"),
            Self::Protocol(e) => write!(f, "PROTOCOL: {e}"),
            Self::Qr(e) => write!(f, "QR: {e}"),
            Self::WebRtc(e) => write!(f, "WEBRTC: {e}"),
            Self::Auth(e) => write!(f, "AUTH: {e}"),
        }
    }
}

impl std::error::Error for VchatError {}

impl From<anyhow::Error> for VchatError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("Operation failed: {e}");
        Self::Protocol(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, VchatError>;

pub fn audit_log(event: &str, details: &str) {
    tracing::info!(
        target: "audit",
        timestamp = %chrono::Utc::now().to_rfc3339(),
        event = event,
        details = details,
        "AUDIT"
    );
}
