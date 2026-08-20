pub mod protocol;

use anyhow::{bail, Result};
use crate::commands::{
    Contact, FileTransfer, Group, GroupMember, GroupMessage, Message, MessageType, Reaction,
};
use crate::crypto;
use crate::crypto::store;
use crate::messaging::protocol::{
    create_wire_message, payload_to_json, serialize_wire_message, DeliveryReceiptPayload,
    GroupCreatePayload, GroupMemberInfo, GroupMessagePayload, ReactionPayload,
    ReadReceiptPayload, TextPayload, TypingPayload, WireMessageType,
};
use crate::error::audit_log;
use tracing::{debug, info, warn};

// ── Helpers ─────────────────────────────────────────────────────────────────

fn message_type_str(mt: &MessageType) -> &'static str {
    mt.as_str()
}

fn parse_message_type(s: &str) -> MessageType {
    MessageType::from_string(s)
}

async fn load_identity_or_bail() -> Result<crate::commands::Identity> {
    store::load_identity()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Identity not initialized"))
}

async fn load_signing_key_or_bail() -> Result<crate::crypto::keys::Ed25519KeyPair> {
    crypto::load_signing_key()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Signing key not found"))
}

async fn load_static_secret_or_bail() -> Result<x25519_dalek::StaticSecret> {
    crypto::load_static_secret()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Static secret not found"))
}

fn resolve_contact_pubkey(contact: &Contact) -> Result<x25519_dalek::PublicKey> {
    let raw: [u8; 32] = hex::decode(&contact.public_key)
        .map_err(|e| anyhow::anyhow!("Invalid contact public key hex: {e}"))?
        .get(..32)
        .ok_or_else(|| anyhow::anyhow!("Public key too short (need 32 bytes)"))?
        .try_into()?;
    Ok(x25519_dalek::PublicKey::from(raw))
}

fn find_contact<'a>(contacts: &'a [Contact], onion: &str) -> Result<&'a Contact> {
    contacts
        .iter()
        .find(|c| c.onion_address == onion)
        .ok_or_else(|| anyhow::anyhow!("Contact not found: {onion}"))
}

async fn try_send_wire(onion: &str, wire_bytes: &[u8]) {
    match crate::tor::connect_to_peer(onion, 4433).await {
        Ok(mut stream) => {
            use tokio::io::AsyncWriteExt;
            match stream.write_all(wire_bytes).await {
                Ok(()) => info!("Wire message delivered to {onion}"),
                Err(e) => warn!("Write failed to {onion}: {e} (message stored locally)"),
            }
        }
        Err(e) => warn!("Cannot reach {onion}: {e} (message stored locally for retry)"),
    }
}

async fn find_peer_for_message(message_id: &str) -> Result<Option<String>> {
    let identity = load_identity_or_bail().await.ok();
    let my_onion = identity.as_ref().map(|i| i.onion_address.as_str()).unwrap_or("");
    let contacts = store::load_contacts().await?;
    for contact in &contacts {
        let msgs = store::load_messages(&contact.onion_address, 500, 0).await?;
        if msgs.iter().any(|m| m.id == message_id) {
            return Ok(Some(contact.onion_address.clone()));
        }
    }
    let _ = my_onion;
    Ok(None)
}

// ── Contacts ────────────────────────────────────────────────────────────────

pub async fn add_contact(
    display_name: &str,
    public_key: &str,
    onion_address: &str,
) -> Result<Contact> {
    let key_bytes = hex::decode(public_key)?;
    if key_bytes.len() != 64 {
        bail!(
            "Public key must be 64 hex-encoded bytes (x25519 + ed25519), got {}",
            key_bytes.len()
        );
    }

    let ed25519_bytes: [u8; 32] = key_bytes[32..64].try_into()?;
    let expected_onion = crypto::generate_onion_from_pubkey(&ed25519_bytes);

    if expected_onion != onion_address {
        warn!(
            "Onion mismatch: expected={expected_onion}, got={onion_address}. \
             Contact may be tampered with or key rotation occurred."
        );
    }

    let now = chrono::Utc::now().timestamp();
    let contact = Contact {
        id: uuid::Uuid::new_v4().to_string(),
        display_name: display_name.to_string(),
        public_key: public_key.to_string(),
        onion_address: onion_address.to_string(),
        added_at: now,
        verified: false,
        blocked: false,
    };

    store::save_contact(&contact).await?;
    audit_log("contact_added", &format!("name={display_name}, onion={onion_address}"));
    info!("Contact added: {display_name} ({onion_address})");

    Ok(contact)
}

pub async fn get_contacts() -> Result<Vec<Contact>> {
    store::load_contacts().await
}

// ── Messages ────────────────────────────────────────────────────────────────

pub async fn send_message(
    recipient_onion: &str,
    content: &str,
    message_type: MessageType,
) -> Result<Message> {
    let identity = load_identity_or_bail().await?;
    let signing_key = load_signing_key_or_bail().await?;
    let static_secret = load_static_secret_or_bail().await?;

    let contacts = store::load_contacts().await?;
    let contact = find_contact(&contacts, recipient_onion)?;

    let their_pub = resolve_contact_pubkey(contact)?;
    let shared_key = crypto::derive_shared_key(&static_secret, &their_pub);
    let encrypted = crypto::encrypt_message(&shared_key, content.as_bytes())?;

    let seq = store::get_message_count(recipient_onion).await? + 1;
    let msg_id = uuid::Uuid::new_v4().to_string();

    let text_payload = TextPayload {
        content: content.to_string(),
        reply_to: None,
    };
    let payload_bytes = payload_to_json(&text_payload)?;

    let wire_msg = create_wire_message(
        &signing_key.signing_key(),
        &signing_key.verifying_key,
        WireMessageType::Text,
        payload_bytes,
        msg_id.clone(),
        seq as u64,
    )?;
    let wire_bytes = serialize_wire_message(&wire_msg)?;

    let timestamp = chrono::Utc::now().timestamp();

    let message = Message {
        id: msg_id,
        sender: identity.onion_address.clone(),
        recipient: recipient_onion.to_string(),
        content: content.to_string(),
        timestamp,
        encrypted: true,
        message_type,
        sequence_num: seq,
        reply_to: None,
        delivered: false,
        read: false,
        expires_at: None,
    };

    store::save_message_with_encrypted(
        &message.id,
        &message.sender,
        &message.recipient,
        Some(&message.content),
        Some(&encrypted),
        message.timestamp,
        message_type_str(&message.message_type),
        "sent",
        Some(message.sequence_num),
        message.reply_to.as_deref(),
    )
    .await?;

    try_send_wire(recipient_onion, &wire_bytes).await;

    audit_log(
        "message_sent",
        &format!("to={recipient_onion}, seq={seq}, id={}", message.id),
    );

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

    let identity = load_identity_or_bail().await?;
    let seq = store::get_message_count(sender_onion).await? + 1;
    let msg_id = uuid::Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().timestamp();

    store::save_message_with_encrypted(
        &msg_id,
        sender_onion,
        &identity.onion_address,
        Some(&content),
        Some(encrypted_content),
        timestamp,
        "text",
        "received",
        Some(seq),
        None,
    )
    .await?;

    let message = Message {
        id: msg_id,
        sender: sender_onion.to_string(),
        recipient: identity.onion_address,
        content,
        timestamp,
        encrypted: true,
        message_type: MessageType::Text,
        sequence_num: seq,
        reply_to: None,
        delivered: true,
        read: false,
        expires_at: None,
    };

    audit_log("message_received", &format!("from={sender_onion}, seq={seq}"));
    debug!("Decrypted and stored message from {sender_onion}");

    Ok(message)
}

pub async fn get_messages(contact_onion: &str) -> Result<Vec<Message>> {
    store::load_messages(contact_onion, 500, 0).await
}

pub async fn delete_message(message_id: &str) -> Result<()> {
    store::delete_message(message_id).await?;
    audit_log("message_deleted", &format!("id={message_id}"));
    info!("Message deleted: {message_id}");
    Ok(())
}

pub async fn search_messages(query: &str) -> Result<Vec<Message>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    store::search_messages(query, 200).await
}

// ── Reactions ───────────────────────────────────────────────────────────────

pub async fn send_reaction(message_id: &str, emoji: &str) -> Result<Reaction> {
    let identity = load_identity_or_bail().await?;
    let reaction_id = uuid::Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().timestamp();

    store::save_reaction(&reaction_id, message_id, &identity.onion_address, emoji).await?;

    let reaction = Reaction {
        id: reaction_id.clone(),
        message_id: message_id.to_string(),
        sender: identity.onion_address.clone(),
        emoji: emoji.to_string(),
        timestamp,
    };

    if let Some(peer_onion) = find_peer_for_message(message_id).await? {
        let signing_key = load_signing_key_or_bail().await?;
        let reaction_payload = ReactionPayload {
            message_id: message_id.to_string(),
            emoji: emoji.to_string(),
        };
        let payload_bytes = payload_to_json(&reaction_payload)?;

        match create_wire_message(
            &signing_key.signing_key(),
            &signing_key.verifying_key,
            WireMessageType::Reaction,
            payload_bytes,
            reaction_id,
            0,
        ) {
            Ok(wire_msg) => match serialize_wire_message(&wire_msg) {
                Ok(wire_bytes) => try_send_wire(&peer_onion, &wire_bytes).await,
                Err(e) => warn!("Failed to serialize reaction wire message: {e}"),
            },
            Err(e) => warn!("Failed to create reaction wire message: {e}"),
        }
    } else {
        debug!("Could not resolve peer for message {message_id}, reaction stored locally only");
    }

    audit_log(
        "reaction_sent",
        &format!("message={message_id}, emoji={emoji}"),
    );

    Ok(reaction)
}

pub async fn remove_reaction(message_id: &str, emoji: &str) -> Result<()> {
    let identity = load_identity_or_bail().await?;
    store::remove_reaction(message_id, &identity.onion_address, emoji).await?;
    audit_log(
        "reaction_removed",
        &format!("message={message_id}, emoji={emoji}"),
    );
    Ok(())
}

pub async fn get_reactions(message_id: &str) -> Result<Vec<Reaction>> {
    let rows = store::load_reactions(message_id).await?;
    Ok(rows
        .into_iter()
        .map(|(id, msg_id, sender, emoji, ts)| Reaction {
            id,
            message_id: msg_id,
            sender,
            emoji,
            timestamp: ts,
        })
        .collect())
}

// ── Delivery / Read Status ──────────────────────────────────────────────────

pub async fn mark_delivered(message_ids: &[String]) -> Result<()> {
    for id in message_ids {
        store::mark_message_delivered(id).await?;
    }
    if !message_ids.is_empty() {
        audit_log(
            "messages_delivered",
            &format!("count={}", message_ids.len()),
        );
    }
    Ok(())
}

pub async fn mark_read(message_ids: &[String]) -> Result<()> {
    for id in message_ids {
        store::mark_message_read(id).await?;
    }
    if !message_ids.is_empty() {
        audit_log("messages_read", &format!("count={}", message_ids.len()));
    }
    Ok(())
}

pub async fn send_delivery_receipt(peer_onion: &str, message_ids: &[String]) -> Result<()> {
    let signing_key = load_signing_key_or_bail().await?;

    let payload = DeliveryReceiptPayload {
        message_ids: message_ids.to_vec(),
    };
    let payload_bytes = payload_to_json(&payload)?;

    let wire_msg = create_wire_message(
        &signing_key.signing_key(),
        &signing_key.verifying_key,
        WireMessageType::DeliveryReceipt,
        payload_bytes,
        uuid::Uuid::new_v4().to_string(),
        0,
    )?;
    let wire_bytes = serialize_wire_message(&wire_msg)?;

    try_send_wire(peer_onion, &wire_bytes).await;
    Ok(())
}

pub async fn send_read_receipt(peer_onion: &str, message_ids: &[String]) -> Result<()> {
    let signing_key = load_signing_key_or_bail().await?;

    let payload = ReadReceiptPayload {
        message_ids: message_ids.to_vec(),
    };
    let payload_bytes = payload_to_json(&payload)?;

    let wire_msg = create_wire_message(
        &signing_key.signing_key(),
        &signing_key.verifying_key,
        WireMessageType::ReadReceipt,
        payload_bytes,
        uuid::Uuid::new_v4().to_string(),
        0,
    )?;
    let wire_bytes = serialize_wire_message(&wire_msg)?;

    try_send_wire(peer_onion, &wire_bytes).await;
    Ok(())
}

// ── Typing Indicator ────────────────────────────────────────────────────────

pub async fn send_typing_indicator(peer_onion: &str, is_typing: bool) -> Result<()> {
    let signing_key = load_signing_key_or_bail().await?;

    let payload = TypingPayload {
        peer_onion: peer_onion.to_string(),
        is_typing,
    };
    let payload_bytes = payload_to_json(&payload)?;

    let wire_msg = create_wire_message(
        &signing_key.signing_key(),
        &signing_key.verifying_key,
        WireMessageType::TypingIndicator,
        payload_bytes,
        uuid::Uuid::new_v4().to_string(),
        0,
    )?;
    let wire_bytes = serialize_wire_message(&wire_msg)?;

    try_send_wire(peer_onion, &wire_bytes).await;

    debug!(
        "Typing indicator (typing={is_typing}) sent to {peer_onion}"
    );

    Ok(())
}

// ── Disappearing Messages ───────────────────────────────────────────────────

pub async fn set_disappearing_message(message_id: &str, ttl_secs: u64) -> Result<()> {
    let expires_at = chrono::Utc::now().timestamp() + ttl_secs as i64;

    store::set_setting(
        &format!("disappear:{message_id}"),
        &ttl_secs.to_string(),
    )
    .await?;

    let setting_key = format!("expires:{message_id}");
    store::set_setting(&setting_key, &expires_at.to_string()).await?;

    audit_log(
        "disappearing_message_set",
        &format!("message={message_id}, ttl={ttl_secs}s"),
    );
    info!(
        "Message {message_id} set to disappear in {ttl_secs}s (expires at {expires_at})"
    );

    Ok(())
}

// ── Groups ──────────────────────────────────────────────────────────────────

pub async fn create_group(
    name: &str,
    description: Option<&str>,
    member_onions: &[String],
) -> Result<Group> {
    let identity = load_identity_or_bail().await?;
    let signing_key = load_signing_key_or_bail().await?;
    let group_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    store::save_group(&group_id, name, description, None, &identity.onion_address).await?;
    store::add_group_member(
        &group_id,
        &identity.onion_address,
        Some(&identity.public_key),
        Some(&identity.display_name),
        "admin",
    )
    .await?;

    let mut members: Vec<GroupMember> = vec![GroupMember {
        group_id: group_id.clone(),
        onion_address: identity.onion_address.clone(),
        public_key: identity.public_key.clone(),
        display_name: identity.display_name.clone(),
        role: "admin".to_string(),
        joined_at: now,
    }];

    let contacts = store::load_contacts().await?;

    for member_onion in member_onions {
        match find_contact(&contacts, member_onion) {
            Ok(contact) => {
                store::add_group_member(
                    &group_id,
                    &contact.onion_address,
                    Some(&contact.public_key),
                    Some(&contact.display_name),
                    "member",
                )
                .await?;

                members.push(GroupMember {
                    group_id: group_id.clone(),
                    onion_address: contact.onion_address.clone(),
                    public_key: contact.public_key.clone(),
                    display_name: contact.display_name.clone(),
                    role: "member".to_string(),
                    joined_at: now,
                });

                let create_payload = GroupCreatePayload {
                    group_id: group_id.clone(),
                    name: name.to_string(),
                    members: members
                        .iter()
                        .map(|m| GroupMemberInfo {
                            onion_address: m.onion_address.clone(),
                            public_key: m.public_key.clone(),
                            display_name: m.display_name.clone(),
                            role: m.role.clone(),
                        })
                        .collect(),
                };
                let payload_bytes = payload_to_json(&create_payload)?;

                match create_wire_message(
                    &signing_key.signing_key(),
                    &signing_key.verifying_key,
                    WireMessageType::GroupCreate,
                    payload_bytes,
                    uuid::Uuid::new_v4().to_string(),
                    0,
                ) {
                    Ok(wire_msg) => match serialize_wire_message(&wire_msg) {
                        Ok(wire_bytes) => {
                            try_send_wire(&contact.onion_address, &wire_bytes).await
                        }
                        Err(e) => warn!("Failed to serialize group create for {}: {e}", contact.onion_address),
                    },
                    Err(e) => warn!("Failed to create wire message for {}: {e}", contact.onion_address),
                }
            }
            Err(e) => {
                warn!("Skipping member {member_onion} for group creation: {e}");
            }
        }
    }

    let group = Group {
        id: group_id.clone(),
        name: name.to_string(),
        description: description.map(|s| s.to_string()).unwrap_or_default(),
        created_by: identity.onion_address,
        created_at: now,
        member_count: members.len() as i64,
    };

    audit_log(
        "group_created",
        &format!("group={group_id}, name={name}, members={}", member_onions.len()),
    );
    info!("Group created: {name} ({group_id})");

    Ok(group)
}

pub async fn get_groups() -> Result<Vec<Group>> {
    let raw_groups = store::load_groups().await?;
    let mut groups = Vec::with_capacity(raw_groups.len());

    for (id, name, description, _avatar, created_by, created_at, _updated_at) in raw_groups {
        let raw_members = store::load_group_members(&id).await?;
        let member_count = raw_members.len() as i64;

        groups.push(Group {
            id,
            name,
            description: description.unwrap_or_default(),
            created_by,
            created_at,
            member_count,
        });
    }

    Ok(groups)
}

pub async fn send_group_message(
    group_id: &str,
    content: &str,
    message_type: MessageType,
) -> Result<GroupMessage> {
    let identity = load_identity_or_bail().await?;
    let signing_key = load_signing_key_or_bail().await?;

    let msg_id = uuid::Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().timestamp();
    let type_str = message_type_str(&message_type);

    store::save_group_message(
        &msg_id,
        group_id,
        &identity.onion_address,
        Some(content),
        None,
        timestamp,
        type_str,
        None,
        None,
    )
    .await?;

    let group_msg = GroupMessage {
        id: msg_id.clone(),
        group_id: group_id.to_string(),
        sender: identity.onion_address.clone(),
        content: content.to_string(),
        timestamp,
        message_type,
        reply_to: None,
    };

    let members = store::load_group_members(group_id).await?;

    let group_payload = GroupMessagePayload {
        group_id: group_id.to_string(),
        content: content.to_string(),
        reply_to: None,
    };
    let payload_bytes = payload_to_json(&group_payload)?;

    for (_gid, member_onion, _pk, _dn, _role, _joined) in &members {
        if member_onion == &identity.onion_address {
            continue;
        }

        match create_wire_message(
            &signing_key.signing_key(),
            &signing_key.verifying_key,
            WireMessageType::GroupMessage,
            payload_bytes.clone(),
            uuid::Uuid::new_v4().to_string(),
            0,
        ) {
            Ok(wire_msg) => match serialize_wire_message(&wire_msg) {
                Ok(wire_bytes) => try_send_wire(member_onion, &wire_bytes).await,
                Err(e) => warn!("Serialize group msg for {member_onion}: {e}"),
            },
            Err(e) => warn!("Create wire msg for {member_onion}: {e}"),
        }
    }

    audit_log(
        "group_message_sent",
        &format!("group={group_id}, id={msg_id}"),
    );

    Ok(group_msg)
}

pub async fn get_group_messages(group_id: &str) -> Result<Vec<GroupMessage>> {
    let raw = store::load_group_messages(group_id, 500, 0).await?;
    Ok(raw
        .into_iter()
        .map(|(id, gid, sender, content, _enc, ts, mt, _seq, reply)| GroupMessage {
            id,
            group_id: gid,
            sender,
            content: content.unwrap_or_default(),
            timestamp: ts,
            message_type: parse_message_type(&mt),
            reply_to: reply,
        })
        .collect())
}

pub async fn add_group_member(
    group_id: &str,
    onion_address: &str,
    public_key: &str,
    display_name: &str,
) -> Result<()> {
    let signing_key = load_signing_key_or_bail().await?;

    store::add_group_member(group_id, onion_address, Some(public_key), Some(display_name), "member")
        .await?;

    let update_payload = crate::messaging::protocol::GroupUpdatePayload {
        group_id: group_id.to_string(),
        update_type: "member_added".to_string(),
        data: serde_json::json!({
            "onion_address": onion_address,
            "display_name": display_name,
        }),
    };
    let payload_bytes = payload_to_json(&update_payload)?;

    match create_wire_message(
        &signing_key.signing_key(),
        &signing_key.verifying_key,
        WireMessageType::GroupUpdate,
        payload_bytes,
        uuid::Uuid::new_v4().to_string(),
        0,
    ) {
        Ok(wire_msg) => match serialize_wire_message(&wire_msg) {
            Ok(wire_bytes) => try_send_wire(onion_address, &wire_bytes).await,
            Err(e) => warn!("Serialize group update: {e}"),
        },
        Err(e) => warn!("Create group update wire msg: {e}"),
    }

    audit_log(
        "group_member_added",
        &format!("group={group_id}, member={onion_address}"),
    );
    info!("Added {onion_address} to group {group_id}");

    Ok(())
}

pub async fn remove_group_member(group_id: &str, onion_address: &str) -> Result<()> {
    let signing_key = load_signing_key_or_bail().await?;

    store::remove_group_member(group_id, onion_address).await?;

    let update_payload = crate::messaging::protocol::GroupUpdatePayload {
        group_id: group_id.to_string(),
        update_type: "member_removed".to_string(),
        data: serde_json::json!({
            "onion_address": onion_address,
        }),
    };
    let payload_bytes = payload_to_json(&update_payload)?;

    match create_wire_message(
        &signing_key.signing_key(),
        &signing_key.verifying_key,
        WireMessageType::GroupUpdate,
        payload_bytes,
        uuid::Uuid::new_v4().to_string(),
        0,
    ) {
        Ok(wire_msg) => match serialize_wire_message(&wire_msg) {
            Ok(wire_bytes) => try_send_wire(onion_address, &wire_bytes).await,
            Err(e) => warn!("Serialize group removal: {e}"),
        },
        Err(e) => warn!("Create group removal wire msg: {e}"),
    }

    audit_log(
        "group_member_removed",
        &format!("group={group_id}, member={onion_address}"),
    );
    info!("Removed {onion_address} from group {group_id}");

    Ok(())
}

// ── File Transfers ──────────────────────────────────────────────────────────

pub async fn save_file_transfer(
    sender: &str,
    recipient: &str,
    filename: &str,
    mime_type: Option<&str>,
    size: Option<i64>,
) -> Result<FileTransfer> {
    let id = uuid::Uuid::new_v4().to_string();

    store::save_file_transfer(
        &id,
        sender,
        recipient,
        filename,
        mime_type,
        size,
        None,
        None,
        "pending",
    )
    .await?;

    let transfer = FileTransfer {
        id: id.clone(),
        sender: sender.to_string(),
        recipient: recipient.to_string(),
        filename: filename.to_string(),
        mime_type: mime_type.unwrap_or_default().to_string(),
        size: size.unwrap_or(0),
        status: "pending".to_string(),
        started_at: chrono::Utc::now().timestamp(),
        completed_at: None,
    };

    audit_log(
        "file_transfer_started",
        &format!("id={id}, from={sender}, to={recipient}, file={filename}"),
    );

    Ok(transfer)
}

pub async fn get_file_transfers() -> Result<Vec<FileTransfer>> {
    let raw = store::load_file_transfers(None, 200, 0).await?;
    Ok(raw
        .into_iter()
        .map(
            |(id, sender, recipient, filename, mime, size, _enc, _dir, status, started, completed)| {
                FileTransfer {
                    id,
                    sender,
                    recipient,
                    filename,
                    mime_type: mime.unwrap_or_default(),
                    size: size.unwrap_or(0),
                    status,
                    started_at: started,
                    completed_at: completed,
                }
            },
        )
        .collect())
}

// ── Cleanup ─────────────────────────────────────────────────────────────────

pub async fn cleanup_expired_messages() -> Result<()> {
    let expired_count = store::cleanup_expired_messages(0).await?;
    if expired_count > 0 {
        info!("Cleaned up {expired_count} expired messages");
    }

    let stale_indicators = store::cleanup_typing_indicators(60).await?;
    if stale_indicators > 0 {
        debug!("Cleaned up {stale_indicators} stale typing indicators");
    }

    let now = chrono::Utc::now().timestamp();

    let contacts = store::load_contacts().await?;
    let mut cleaned = 0u64;

    for contact in &contacts {
        let msgs = store::load_messages(&contact.onion_address, 500, 0).await?;
        for msg in &msgs {
            if let Some(expires_at) = msg.expires_at {
                if expires_at <= now {
                    if let Err(e) = store::delete_message(&msg.id).await {
                        warn!("Failed to delete expired message {}: {e}", msg.id);
                    } else {
                        cleaned += 1;
                    }
                }
            }
        }
    }

    if cleaned > 0 {
        info!("Removed {cleaned} disappearing messages past TTL");
        audit_log("disappearing_cleanup", &format!("removed={cleaned}"));
    }

    Ok(())
}
