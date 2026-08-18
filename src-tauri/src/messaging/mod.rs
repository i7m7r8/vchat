pub mod protocol;

use anyhow::Result;
use crate::commands::{Contact, Message, MessageType};
use crate::crypto;
use crate::crypto::store;
use crate::messaging::protocol::{
    create_wire_message, serialize_wire_message, TextPayload, WireMessageType,
};
use tracing::{info, warn};

pub async fn send_message(
    recipient_onion: &str,
    content: &str,
    message_type: MessageType,
) -> Result<Message> {
    let identity = store::load_identity()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Identity not initialized"))?;

    let signing_key = crypto::load_signing_key()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Signing key not found"))?;

    let static_secret = crypto::load_static_secret()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Static secret not found"))?;

    let contacts = store::load_contacts().await?;
    let contact = contacts
        .iter()
        .find(|c| c.onion_address == recipient_onion)
        .ok_or_else(|| anyhow::anyhow!("Contact not found: {recipient_onion}"))?;

    let their_x25519_pubkey_bytes: [u8; 32] = hex::decode(&contact.public_key)
        .map_err(|e| anyhow::anyhow!("Invalid contact public key: {e}"))?
        .get(..32)
        .ok_or_else(|| anyhow::anyhow!("Public key too short"))?
        .try_into()?;

    let their_x25519_pub = x25519_dalek::PublicKey::from(their_x25519_pubkey_bytes);
    let shared_key = crypto::derive_shared_key(&static_secret, &their_x25519_pub);

    let plaintext = content.as_bytes();
    let encrypted = crypto::encrypt_message(&shared_key, plaintext)?;

    let seq = store::get_message_count(recipient_onion).await? as u64 + 1;

    let text_payload = TextPayload {
        content: content.to_string(),
        reply_to: None,
        ephemeral_ttl: None,
    };
    let payload_bytes = serde_json::to_vec(&text_payload)?;

    let wire_msg = create_wire_message(
        WireMessageType::TextMessage,
        payload_bytes,
        &signing_key.public_key_bytes(),
        &signing_key.signing_key(),
        seq,
    )?;

    let _wire_bytes = serialize_wire_message(&wire_msg)?;

    let message = Message {
        id: uuid::Uuid::new_v4().to_string(),
        sender: identity.onion_address.clone(),
        recipient: recipient_onion.to_string(),
        content: content.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        encrypted: true,
        message_type,
        sequence_num: seq as i64,
    };

    store::save_message_with_encrypted(&message, &encrypted).await?;

    match crate::tor::connect_to_peer(recipient_onion, 4433).await {
        Ok(mut stream) => {
            use tokio::io::AsyncWriteExt;
            match stream.write_all(&_wire_bytes).await {
                Ok(()) => {
                    info!("Message sent to {recipient_onion}");
                    crate::error::audit_log(
                        "message_sent",
                        &format!("to={recipient_onion}, seq={seq}"),
                    );
                }
                Err(e) => {
                    warn!("Failed to send to {recipient_onion}: {e} (message stored locally)");
                }
            }
        }
        Err(e) => {
            warn!("Cannot reach {recipient_onion}: {e} (message stored locally for retry)");
        }
    }

    Ok(message)
}

pub async fn receive_message(
    sender_onion: &str,
    encrypted_content: &[u8],
    shared_key: &[u8; 32],
) -> Result<Message> {
    let decrypted = crypto::decrypt_message(shared_key, encrypted_content)?;
    let content = String::from_utf8(decrypted)
        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in decrypted message: {e}"))?;

    let identity = store::load_identity()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Identity not initialized"))?;

    let seq = store::get_message_count(sender_onion).await? as i64 + 1;

    let message = Message {
        id: uuid::Uuid::new_v4().to_string(),
        sender: sender_onion.to_string(),
        recipient: identity.onion_address,
        content,
        timestamp: chrono::Utc::now().timestamp(),
        encrypted: true,
        message_type: MessageType::Text,
        sequence_num: seq,
    };

    store::save_message(&message).await?;

    crate::error::audit_log(
        "message_received",
        &format!("from={sender_onion}, seq={seq}"),
    );

    Ok(message)
}

pub async fn add_contact(
    display_name: &str,
    public_key: &str,
    onion_address: &str,
) -> Result<Contact> {
    let key_bytes = hex::decode(public_key)?;
    if key_bytes.len() != 64 {
        anyhow::bail!("Public key must be 64 bytes (x25519 + ed25519), got {}", key_bytes.len());
    }

    let x25519_bytes: [u8; 32] = key_bytes[..32].try_into()?;
    let expected_onion = crypto::generate_onion_from_pubkey(&x25519_bytes);

    if expected_onion != onion_address {
        warn!(
            "Onion address mismatch: expected={expected_onion}, got={onion_address}. Contact may be tampered."
        );
    }

    let contact = Contact {
        id: uuid::Uuid::new_v4().to_string(),
        display_name: display_name.to_string(),
        public_key: public_key.to_string(),
        onion_address: onion_address.to_string(),
        added_at: chrono::Utc::now().timestamp(),
    };

    store::save_contact(&contact).await?;

    crate::error::audit_log(
        "contact_added",
        &format!("name={display_name}, onion={onion_address}"),
    );

    Ok(contact)
}

pub async fn get_contacts() -> Result<Vec<Contact>> {
    store::load_contacts().await
}

pub async fn get_messages(contact_onion: &str) -> Result<Vec<Message>> {
    store::load_messages(contact_onion).await
}
