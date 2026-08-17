pub mod keys;
pub mod noise;
pub mod store;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::commands::{Contact, Identity};

pub async fn generate_identity(display_name: &str) -> Result<Identity> {
    let static_secret = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
    let static_public = x25519_dalek::PublicKey::from(&static_secret);

    let onion_hash = Sha256::digest(static_public.as_bytes());
    let onion_address = format!(
        "{}.onion",
        crate::base32::encode_base32(&onion_hash[..])
            .to_lowercase()
    );

    let identity = Identity {
        public_key: hex::encode(static_public.as_bytes()),
        onion_address,
        display_name: display_name.to_string(),
    };

    store::save_identity(&identity).await?;

    Ok(identity)
}

pub async fn load_identity() -> Result<Option<Identity>> {
    store::load_identity().await
}

pub async fn generate_qr_code() -> Result<String> {
    let identity = load_identity()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Identity not initialized"))?;

    let qr_data = serde_json::to_string(&serde_json::json!({
        "public_key": identity.public_key,
        "onion_address": identity.onion_address,
        "display_name": identity.display_name,
    }))?;

    Ok(qr_data)
}

pub async fn scan_qr_code(qr_data: &str) -> Result<Contact> {
    let data: serde_json::Value = serde_json::from_str(qr_data)?;

    let contact = Contact {
        id: uuid::Uuid::new_v4().to_string(),
        display_name: data["display_name"]
            .as_str()
            .unwrap_or("Unknown")
            .to_string(),
        public_key: data["public_key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing public_key"))?
            .to_string(),
        onion_address: data["onion_address"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing onion_address"))?
            .to_string(),
        added_at: chrono::Utc::now().timestamp(),
    };

    Ok(contact)
}

pub fn encrypt_message(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };

    let cipher = Aes256Gcm::new_from_slice(key)?;
    let nonce = Nonce::from_slice(&rand::random::<[u8; 12]>());

    let ciphertext = cipher.encrypt(nonce, plaintext)?;

    let mut result = nonce.to_vec();
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

pub fn decrypt_message(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };

    if ciphertext.len() < 12 {
        anyhow::bail!("Ciphertext too short");
    }

    let cipher = Aes256Gcm::new_from_slice(key)?;
    let nonce = Nonce::from_slice(&ciphertext[..12]);

    let plaintext = cipher.decrypt(nonce, &ciphertext[12..])?;

    Ok(plaintext)
}

pub fn derive_shared_key(
    our_secret: &x25519_dalek::StaticSecret,
    their_public: &x25519_dalek::PublicKey,
) -> [u8; 32] {
    let shared_secret = our_secret.diffie_hellman(their_public);
    shared_secret.as_bytes().clone()
}
