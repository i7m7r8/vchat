pub mod keys;
pub mod noise;
pub mod store;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::Result;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;

use crate::commands::{Contact, Identity};

pub const KEY_LENGTH: usize = 32;
pub const NONCE_LENGTH: usize = 12;

#[derive(Zeroize)]
pub struct SecureKey([u8; KEY_LENGTH]);

impl SecureKey {
    pub fn new(bytes: [u8; KEY_LENGTH]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; KEY_LENGTH] {
        &self.0
    }
}

impl Drop for SecureKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub fn generate_onion_from_pubkey(pubkey_bytes: &[u8; 32]) -> String {
    crate::tor::hidden_service::pubkey_to_v3_onion(pubkey_bytes)
}

pub async fn generate_identity(display_name: &str) -> Result<Identity> {
    let x25519_kp = keys::X25519KeyPair::generate();
    let ed25519_kp = keys::Ed25519KeyPair::generate()?;

    let onion_address = generate_onion_from_pubkey(&ed25519_kp.public_key_bytes());

    let mut combined_pub = Vec::with_capacity(64);
    combined_pub.extend_from_slice(&x25519_kp.public_bytes());
    combined_pub.extend_from_slice(&ed25519_kp.public_key_bytes());

    let x25519_secret_hex = hex::encode(x25519_kp.secret_bytes());
    let ed25519_secret_hex = ed25519_kp.secret_key_hex();

    let identity = Identity {
        public_key: hex::encode(&combined_pub),
        onion_address,
        display_name: display_name.to_string(),
    };

    store::save_identity_with_keys(
        &identity,
        &x25519_secret_hex,
        &ed25519_secret_hex,
    )
    .await?;

    crate::error::audit_log("identity_created", &format!("onion={}", identity.onion_address));

    Ok(identity)
}

pub async fn load_identity() -> Result<Option<Identity>> {
    store::load_identity().await
}

pub async fn load_signing_key() -> Result<Option<keys::Ed25519KeyPair>> {
    let secret_hex = store::load_ed25519_secret().await?;
    match secret_hex {
        Some(hex_str) => {
            let bytes: [u8; 32] = hex::decode(&hex_str)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid ed25519 key length"))?;
            Ok(Some(keys::Ed25519KeyPair::from_bytes(&bytes)?))
        }
        None => Ok(None),
    }
}

pub async fn load_static_secret() -> Result<Option<x25519_dalek::StaticSecret>> {
    let secret_hex = store::load_x25519_secret().await?;
    match secret_hex {
        Some(hex_str) => {
            let bytes: [u8; 32] = hex::decode(&hex_str)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid x25519 key length"))?;
            Ok(Some(x25519_dalek::StaticSecret::from(bytes)))
        }
        None => Ok(None),
    }
}

pub fn derive_shared_key(
    our_secret: &x25519_dalek::StaticSecret,
    their_public: &x25519_dalek::PublicKey,
) -> [u8; KEY_LENGTH] {
    let shared = our_secret.diffie_hellman(their_public);
    let shared_bytes = *shared.as_bytes();

    let hk = Hkdf::<Sha256>::new(Some(b"vchat-key-derivation-v1"), &shared_bytes);
    let mut derived = [0u8; KEY_LENGTH];
    hk.expand(b"message-encryption-key", &mut derived)
        .expect("HKDF expand failed (output too long for SHA256)");
    derived
}

pub fn encrypt_message(key: &[u8; KEY_LENGTH], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("Cipher creation failed: {e}"))?;

    let nonce_bytes: [u8; NONCE_LENGTH] = rand::random();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("Encryption failed: {e}"))?;

    let mut result = Vec::with_capacity(NONCE_LENGTH + 4 + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&(plaintext.len() as u32).to_le_bytes());
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

pub fn decrypt_message(key: &[u8; KEY_LENGTH], data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < NONCE_LENGTH + 4 + 16 {
        anyhow::bail!("Ciphertext too short: {} bytes", data.len());
    }

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("Cipher creation failed: {e}"))?;

    let nonce = Nonce::from_slice(&data[..NONCE_LENGTH]);

    let plaintext = cipher
        .decrypt(nonce, &data[NONCE_LENGTH + 4..])
        .map_err(|_| anyhow::anyhow!("Decryption failed (wrong key or tampered data)"))?;

    Ok(plaintext)
}

pub async fn generate_qr_code() -> Result<String> {
    let identity = load_identity()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Identity not initialized"))?;

    let qr_payload = serde_json::to_string(&serde_json::json!({
        "v": "1",
        "type": "vchat-contact",
        "name": identity.display_name,
        "key": identity.public_key,
        "onion": identity.onion_address,
    }))?;

    use qrcode::QrCode;
    use qrcode::render::svg;

    let code = QrCode::new(qr_payload.as_bytes())
        .map_err(|e| anyhow::anyhow!("QR generation failed: {e}"))?;

    let svg_string = code
        .render::<svg::Color>()
        .min_dimensions(200, 200)
        .build();

    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        svg_string.as_bytes(),
    );

    Ok(format!("data:image/svg+xml;base64,{b64}"))
}

pub fn parse_qr_data(qr_data: &str) -> Result<Contact> {
    let trimmed = qr_data.trim();
    let json_str = if trimmed.starts_with("vchat://") {
        let parsed = url::Url::parse(trimmed)
            .map_err(|e| anyhow::anyhow!("Invalid vchat URI: {e}"))?;
        let params: std::collections::HashMap<String, String> =
            parsed.query_pairs().into_owned().collect();
        serde_json::to_string(&serde_json::json!({
            "name": params.get("name").cloned().unwrap_or_else(|| "Unknown".to_string()),
            "key": params.get("key").ok_or_else(|| anyhow::anyhow!("Missing key in URI"))?,
            "onion": params.get("onion").ok_or_else(|| anyhow::anyhow!("Missing onion in URI"))?,
        }))?
    } else {
        trimmed.to_string()
    };

    let data: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!("Invalid QR data: {e}"))?;

    let public_key = data["key"]
        .as_str()
        .or_else(|| data["public_key"].as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing public key"))?;

    let key_bytes = hex::decode(public_key)
        .map_err(|e| anyhow::anyhow!("Invalid key hex: {e}"))?;
    if key_bytes.len() != 64 {
        anyhow::bail!("Public key must be 64 bytes, got {}", key_bytes.len());
    }

    let now = chrono::Utc::now().timestamp();
    let contact = Contact {
        id: uuid::Uuid::new_v4().to_string(),
        onion_address: data["onion"]
            .as_str()
            .or_else(|| data["onion_address"].as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing onion address"))?
            .to_string(),
        public_key: public_key.to_string(),
        display_name: data["name"]
            .as_str()
            .or_else(|| data["display_name"].as_str())
            .unwrap_or("Unknown")
            .to_string(),
        added_at: now,
        verified: false,
        blocked: false,
    };

    crate::error::audit_log("qr_scanned", &format!("contact={}", contact.display_name));

    Ok(contact)
}
