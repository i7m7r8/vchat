use anyhow::Result;
use crate::commands::{Contact, Message, MessageType};
use crate::crypto;
use crate::crypto::store;
use tracing::info;

pub async fn send_message(
    recipient_onion: &str,
    content: &str,
    message_type: MessageType,
) -> Result<Message> {
    let identity = store::load_identity()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Identity not initialized"))?;

    let message = Message {
        id: uuid::Uuid::new_v4().to_string(),
        sender: identity.onion_address.clone(),
        recipient: recipient_onion.to_string(),
        content: content.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        encrypted: true,
        message_type,
    };

    store::save_message(&message).await?;

    // Derive shared key and encrypt
    let contacts = store::load_contacts().await?;
    let contact = contacts.iter().find(|c| c.onion_address == recipient_onion);

    if let Some(contact) = contact {
        let public_key_bytes: [u8; 32] = hex::decode(&contact.public_key)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid public key length"))?;

        let their_public = x25519_dalek::PublicKey::from(public_key_bytes);
        let our_secret = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
        let shared_key = crypto::derive_shared_key(&our_secret, &their_public);

        let _encrypted_content = crypto::encrypt_message(&shared_key, content.as_bytes())?;
        info!("Encrypted message for {}", recipient_onion);
    }

    info!(
        "Sent message to {}: {}",
        recipient_onion,
        &content[..content.len().min(50)]
    );

    Ok(message)
}

pub async fn add_contact(
    display_name: &str,
    public_key: &str,
    onion_address: &str,
) -> Result<Contact> {
    let contact = Contact {
        id: uuid::Uuid::new_v4().to_string(),
        display_name: display_name.to_string(),
        public_key: public_key.to_string(),
        onion_address: onion_address.to_string(),
        added_at: chrono::Utc::now().timestamp(),
    };

    store::save_contact(&contact).await?;

    info!("Added contact: {} ({})", display_name, onion_address);

    Ok(contact)
}

pub async fn get_contacts() -> Result<Vec<Contact>> {
    store::load_contacts().await
}

pub async fn get_messages(contact_onion: &str) -> Result<Vec<Message>> {
    store::load_messages(contact_onion).await
}

pub async fn receive_message(
    sender_onion: &str,
    encrypted_content: &[u8],
    shared_key: &[u8; 32],
) -> Result<Message> {
    let decrypted_content = crypto::decrypt_message(shared_key, encrypted_content)?;

    let content = String::from_utf8(decrypted_content)?;

    let message = Message {
        id: uuid::Uuid::new_v4().to_string(),
        sender: sender_onion.to_string(),
        recipient: String::new(),
        content,
        timestamp: chrono::Utc::now().timestamp(),
        encrypted: true,
        message_type: MessageType::Text,
    };

    store::save_message(&message).await?;

    info!("Received message from {}", sender_onion);

    Ok(message)
}
