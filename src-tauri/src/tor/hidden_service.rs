use once_cell::sync::OnceCell;
use sha2::{Digest, Sha512};
use std::net::TcpListener as StdTcpListener;
use tauri::Emitter;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use crate::messaging::protocol::{
    deserialize_wire_message, serialize_wire_message, CallInvitePayload, HeartbeatPayload,
    TextPayload, WireMessage, WireMessageType,
};

static APP_HANDLE: OnceCell<tauri::AppHandle> = OnceCell::new();

pub fn set_app_handle(handle: tauri::AppHandle) {
    APP_HANDLE.set(handle).ok();
}

pub async fn generate_v3_onion_address() -> anyhow::Result<String> {
    let signing_key = crate::crypto::load_signing_key()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Signing key not available for onion generation"))?;
    let pubkey = signing_key.public_key_bytes();
    Ok(pubkey_to_v3_onion(&pubkey))
}

pub fn pubkey_to_v3_onion(pubkey_bytes: &[u8; 32]) -> String {
    let mut version_hasher = Sha512::new();
    version_hasher.update(pubkey_bytes);
    version_hasher.update([0x03u8]);
    let _version_hash = version_hasher.finalize();

    let mut checksum_input = Vec::new();
    checksum_input.extend_from_slice(b".onion checksum");
    checksum_input.extend_from_slice(pubkey_bytes);
    checksum_input.push(0x03);

    let mut checksum_hasher = Sha512::new();
    checksum_hasher.update(&checksum_input);
    let checksum = checksum_hasher.finalize();

    let mut onion_data = Vec::with_capacity(35);
    onion_data.extend_from_slice(&checksum[..2]);
    onion_data.extend_from_slice(pubkey_bytes);
    onion_data.push(0x03);

    let encoded = crate::base32::encode_base32(&onion_data);
    format!("{}.onion", encoded.to_lowercase())
}

pub fn derive_onion_from_keypair(ed25519_secret: &[u8; 32]) -> (String, [u8; 32]) {
    let secret = ed25519_dalek::SigningKey::from_bytes(ed25519_secret);
    let verifying = secret.verifying_key();
    let pubkey = verifying.to_bytes();
    let onion = pubkey_to_v3_onion(&pubkey);
    (onion, pubkey)
}

pub fn verify_onion_matches_pubkey(onion: &str, pubkey: &[u8; 32]) -> bool {
    let expected = pubkey_to_v3_onion(pubkey);
    expected == onion
}

pub async fn find_available_port() -> Result<u16, std::io::Error> {
    for port in 49152..65535 {
        let addr = format!("127.0.0.1:{port}");
        if StdTcpListener::bind(&addr).is_ok() {
            return Ok(port);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        "No available ports in range 49152-65535",
    ))
}

pub async fn accept_loop(listener: TcpListener) {
    info!("Hidden service accepting connections");
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!("Incoming connection from {addr}");
                tokio::spawn(handle_connection(stream, addr));
            }
            Err(e) => {
                error!("Accept error: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

async fn handle_connection(mut stream: tokio::net::TcpStream, addr: std::net::SocketAddr) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    info!("Handling connection from {addr}");

    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) => {
            warn!("Failed to read message length from {addr}: {e}");
            return;
        }
    }

    let msg_len = u32::from_le_bytes(len_buf) as usize;
    if msg_len > crate::messaging::protocol::MAX_MESSAGE_SIZE {
        warn!(
            "Message too large from {addr}: {msg_len} bytes (max {})",
            crate::messaging::protocol::MAX_MESSAGE_SIZE
        );
        return;
    }

    let mut msg_buf = vec![0u8; msg_len];
    match stream.read_exact(&mut msg_buf).await {
        Ok(_) => {}
        Err(e) => {
            warn!("Failed to read message payload from {addr}: {e}");
            return;
        }
    }

    let mut full_buf = Vec::with_capacity(4 + msg_len);
    full_buf.extend_from_slice(&len_buf);
    full_buf.extend_from_slice(&msg_buf);

    let wire_msg = match deserialize_wire_message(&full_buf) {
        Ok(msg) => msg,
        Err(e) => {
            warn!("Failed to deserialize WireMessage from {addr}: {e}");
            let _ = write_error_response(&mut stream, "deserialization_failed").await;
            return;
        }
    };

    info!(
        "Received {:?} from {addr}, seq={}",
        wire_msg.msg_type, wire_msg.sequence
    );

    let response = match wire_msg.msg_type {
        WireMessageType::Text => handle_text_message(&wire_msg, &addr).await,
        WireMessageType::CallInvite => handle_call_invite(&wire_msg, &addr).await,
        WireMessageType::CallAccept => handle_call_accept(&wire_msg, &addr).await,
        WireMessageType::CallReject => handle_call_reject(&wire_msg, &addr).await,
        WireMessageType::CallEnd => handle_call_end(&wire_msg, &addr).await,
        WireMessageType::TypingIndicator => handle_typing_indicator(&wire_msg, &addr).await,
        WireMessageType::Heartbeat => handle_heartbeat(&wire_msg, &addr).await,
        WireMessageType::FileMeta => handle_file_meta(&wire_msg, &addr).await,
        WireMessageType::FileChunk => handle_file_chunk(&wire_msg, &addr).await,
        _ => handle_default(&wire_msg, &addr).await,
    };

    match response {
        Ok(resp_bytes) => {
            if let Err(e) = stream.write_all(&resp_bytes).await {
                warn!("Failed to send response to {addr}: {e}");
            }
        }
        Err(e) => {
            warn!("Error building response for {addr}: {e}");
            let _ = write_error_response(&mut stream, "internal_error").await;
        }
    }
}

async fn handle_text_message(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    let text_payload: TextPayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid TextPayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;

    let content = try_decrypt_content(&text_payload, &sender_onion)
        .await
        .unwrap_or_else(|_| text_payload.content.clone());

    let identity = crate::crypto::store::load_identity().await?
        .ok_or_else(|| anyhow::anyhow!("Identity not initialized"))?;

    let message = crate::commands::Message {
        id: msg.message_id.clone(),
        sender: sender_onion.clone(),
        recipient: identity.onion_address,
        content,
        timestamp: msg.timestamp,
        encrypted: true,
        message_type: crate::commands::MessageType::Text,
        sequence_num: msg.sequence as i64,
        reply_to: text_payload.reply_to,
        delivered: false,
        read: false,
        expires_at: None,
    };

    crate::crypto::store::save_message(&message).await?;

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("new-message", &message) {
            warn!("Failed to emit new-message event: {e}");
        }
    }

    info!(
        target: "vchat::messages",
        from = %sender_onion,
        content_len = message.content.len(),
        sequence = msg.sequence,
        "TextMessage processed from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_call_invite(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    let call_payload: CallInvitePayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid CallInvitePayload: {e}"))?;

    tracing::info!(
        target: "vchat::calls",
        call_id = %call_payload.call_id,
        call_type = ?call_payload.call_type,
        from = %hex::encode(&msg.sender_pubkey),
        sequence = msg.sequence,
        "CallInvite received from {addr}"
    );

    crate::error::audit_log(
        "call_invite_received",
        &format!(
            "call_id={}, from={}",
            call_payload.call_id,
            hex::encode(&msg.sender_pubkey)
        ),
    );

    build_ack_response(msg).await
}

async fn handle_call_accept(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    tracing::info!(
        target: "vchat::calls",
        from = %hex::encode(&msg.sender_pubkey),
        sequence = msg.sequence,
        "CallAccept received from {addr}"
    );

    build_ack_response(msg)
}

async fn handle_call_reject(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    tracing::info!(
        target: "vchat::calls",
        from = %hex::encode(&msg.sender_pubkey),
        sequence = msg.sequence,
        "CallReject received from {addr}"
    );

    build_ack_response(msg)
}

async fn handle_call_end(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    tracing::info!(
        target: "vchat::calls",
        from = %hex::encode(&msg.sender_pubkey),
        sequence = msg.sequence,
        "CallEnd received from {addr}"
    );

    build_ack_response(msg)
}

async fn handle_typing_indicator(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    tracing::info!(
        target: "vchat::typing",
        from = %hex::encode(&msg.sender_pubkey),
        sequence = msg.sequence,
        "TypingIndicator received from {addr}"
    );

    build_ack_response(msg)
}

async fn handle_heartbeat(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    let _payload: Option<HeartbeatPayload> = serde_json::from_slice(&msg.payload).ok();

    tracing::debug!(
        target: "vchat::heartbeat",
        from = %hex::encode(&msg.sender_pubkey),
        sequence = msg.sequence,
        "Heartbeat from {addr}"
    );

    let heartbeat_response = HeartbeatPayload {
        relay_hint: None,
        uptime_secs: 0,
        active_sessions: 1,
    };
    let payload_bytes = serde_json::to_vec(&heartbeat_response)?;

    let signing_key = crate::crypto::load_signing_key()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Signing key not available"))?;

    let seq = msg.sequence + 1;
    let response = crate::messaging::protocol::create_wire_message(
        &signing_key.signing_key(),
        &signing_key.verifying_key,
        WireMessageType::Heartbeat,
        payload_bytes,
        uuid::Uuid::new_v4().to_string(),
        seq,
    )?;

    serialize_wire_message(&response)
}

async fn handle_file_meta(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    tracing::info!(
        target: "vchat::files",
        from = %hex::encode(&msg.sender_pubkey),
        sequence = msg.sequence,
        payload_size = msg.payload.len(),
        "FileMeta received from {addr}"
    );

    build_ack_response(msg)
}

async fn handle_file_chunk(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    tracing::debug!(
        target: "vchat::files",
        from = %hex::encode(&msg.sender_pubkey),
        sequence = msg.sequence,
        payload_size = msg.payload.len(),
        "FileChunk received from {addr}"
    );

    build_ack_response(msg)
}

async fn handle_default(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    warn!(
        "Unhandled message type {:?} from {addr}, seq={}",
        msg.msg_type, msg.sequence
    );

    build_ack_response(msg)
}

async fn build_ack_response(msg: &WireMessage) -> anyhow::Result<Vec<u8>> {
    let ack_payload = serde_json::json!({
        "status": "ok",
        "original_seq": msg.sequence,
    });
    let payload_bytes = serde_json::to_vec(&ack_payload)?;

    let signing_key = crate::crypto::load_signing_key()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Signing key not available for ACK"))?;

    let seq = msg.sequence + 1;

    let response = crate::messaging::protocol::create_wire_message(
        &signing_key.signing_key(),
        &signing_key.verifying_key,
        WireMessageType::Ack,
        payload_bytes,
        uuid::Uuid::new_v4().to_string(),
        seq,
    )?;

    serialize_wire_message(&response)
}

fn derive_sender_onion(msg: &WireMessage) -> anyhow::Result<String> {
    if msg.sender_pubkey.len() == 32 {
        let key: [u8; 32] = msg.sender_pubkey[..32]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid sender key"))?;
        Ok(pubkey_to_v3_onion(&key))
    } else {
        anyhow::bail!(
            "Invalid sender public key length: {}",
            msg.sender_pubkey.len()
        )
    }
}

async fn try_decrypt_content(
    text_payload: &TextPayload,
    sender_onion: &str,
) -> anyhow::Result<String> {
    let contact = crate::crypto::store::load_single_contact(sender_onion)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Sender not in contacts"))?;

    if contact.public_key.len() < 64 {
        anyhow::bail!("Contact public key too short for x25519 extraction");
    }

    let their_x25519_hex = &contact.public_key[..64];
    let their_x25519_bytes: [u8; 32] = hex::decode(their_x25519_hex)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid x25519 key length"))?;

    let their_x25519_pub = x25519_dalek::PublicKey::from(their_x25519_bytes);
    let our_secret = crate::crypto::load_static_secret()
        .await?
        .ok_or_else(|| anyhow::anyhow!("No x25519 static secret available"))?;

    let shared_key = crate::crypto::derive_shared_key(&our_secret, &their_x25519_pub);
    let ciphertext = hex::decode(&text_payload.content)
        .map_err(|e| anyhow::anyhow!("Failed to decode ciphertext hex: {e}"))?;

    let plaintext = crate::crypto::decrypt_message(&shared_key, &ciphertext)?;
    String::from_utf8(plaintext)
        .map_err(|e| anyhow::anyhow!("Decrypted content is not valid UTF-8: {e}"))
}

async fn write_error_response(
    stream: &mut tokio::net::TcpStream,
    error: &str,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;

    let error_msg = serde_json::json!({
        "version": crate::messaging::protocol::WIRE_VERSION,
        "msg_type": "Ack",
        "payload": error.as_bytes(),
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "sequence": 0,
        "sender_pubkey": [],
        "signature": [],
    });
    let json_bytes = serde_json::to_vec(&error_msg)?;
    let len = json_bytes.len() as u32;
    let mut buf = Vec::with_capacity(4 + json_bytes.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&json_bytes);
    stream.write_all(&buf).await?;
    Ok(())
}
