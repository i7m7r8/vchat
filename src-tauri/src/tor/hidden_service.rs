use once_cell::sync::OnceCell;
use once_cell::sync::Lazy;
use sha3::Sha3_256;
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::Emitter;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use crate::messaging::protocol::{
    deserialize_wire_message, serialize_wire_message, CallInvitePayload, HeartbeatPayload,
    TextPayload, TypingPayload, WireMessage, WireMessageType,
};

static APP_HANDLE: OnceCell<tauri::AppHandle> = OnceCell::new();
static APP_START_TIME: Lazy<AtomicU64> = Lazy::new(|| {
    AtomicU64::new(chrono::Utc::now().timestamp() as u64)
});

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
    let mut version_hasher = Sha3_256::new();
    version_hasher.update(pubkey_bytes);
    version_hasher.update([0x03u8]);
    let _version_hash = version_hasher.finalize();

    let mut checksum_input = Vec::new();
    checksum_input.extend_from_slice(b".onion checksum");
    checksum_input.extend_from_slice(pubkey_bytes);
    checksum_input.push(0x03);

    let mut checksum_hasher = Sha3_256::new();
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
        WireMessageType::Reaction => handle_reaction(&wire_msg, &addr).await,
        WireMessageType::DeliveryReceipt => handle_delivery_receipt(&wire_msg, &addr).await,
        WireMessageType::ReadReceipt => handle_read_receipt(&wire_msg, &addr).await,
        WireMessageType::GroupCreate => handle_group_create(&wire_msg, &addr).await,
        WireMessageType::GroupMessage => handle_group_message(&wire_msg, &addr).await,
        WireMessageType::GroupUpdate => handle_group_update(&wire_msg, &addr).await,
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

        if let Err(e) = app.emit("notification", serde_json::json!({
            "type": "message",
            "title": "New Message",
            "body": &message.content[..message.content.len().min(100)],
            "sender": sender_onion,
        })) {
            warn!("Failed to emit notification: {e}");
        }
    }

    if let Err(e) = crate::crypto::store::mark_messages_delivered_by_sender(&sender_onion).await {
        warn!("Failed to mark delivered: {e}");
    }

    let signing_key = crate::crypto::load_signing_key().await?
        .ok_or_else(|| anyhow::anyhow!("Signing key not available"))?;

    let receipt_payload = crate::messaging::protocol::DeliveryReceiptPayload {
        message_ids: vec![msg.message_id.clone()],
    };
    let receipt_bytes = crate::messaging::protocol::payload_to_json(&receipt_payload)?;
    let receipt_wire = crate::messaging::protocol::create_wire_message(
        &signing_key.signing_key(),
        &signing_key.verifying_key,
        WireMessageType::DeliveryReceipt,
        receipt_bytes,
        uuid::Uuid::new_v4().to_string(),
        msg.sequence + 1,
    )?;
    let receipt_wire_bytes = serialize_wire_message(&receipt_wire)?;
    crate::messaging::try_send_wire(&sender_onion, &receipt_wire_bytes).await;

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

    let sender_onion = derive_sender_onion(msg)?;

    crate::crypto::store::save_call_log(
        &call_payload.call_id,
        &sender_onion,
        &format!("{:?}", call_payload.call_type).to_lowercase(),
        "incoming",
        chrono::Utc::now().timestamp(),
        None,
        None,
        "ringing",
    ).await?;

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("incoming-call", serde_json::json!({
            "call_id": call_payload.call_id,
            "caller_onion": sender_onion,
            "call_type": format!("{:?}", call_payload.call_type).to_lowercase(),
            "sdp_offer": call_payload.sdp_offer,
        })) {
            warn!("Failed to emit incoming-call: {e}");
        }
    }

    tracing::info!(
        target: "vchat::calls",
        call_id = %call_payload.call_id,
        call_type = ?call_payload.call_type,
        from = %sender_onion,
        sequence = msg.sequence,
        "CallInvite received from {addr}"
    );

    crate::error::audit_log(
        "call_invite_received",
        &format!("call_id={}, from={}", call_payload.call_id, sender_onion),
    );

    build_ack_response(msg).await
}

async fn handle_call_accept(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    let sender_onion = derive_sender_onion(msg)?;

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("call-accepted", serde_json::json!({
            "sender_onion": sender_onion,
            "message_id": msg.message_id,
        })) {
            warn!("Failed to emit call-accepted: {e}");
        }
    }

    tracing::info!(
        target: "vchat::calls",
        from = %sender_onion,
        sequence = msg.sequence,
        "CallAccept received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_call_reject(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    let sender_onion = derive_sender_onion(msg)?;

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("call-rejected", serde_json::json!({
            "sender_onion": sender_onion,
            "message_id": msg.message_id,
        })) {
            warn!("Failed to emit call-rejected: {e}");
        }
    }

    tracing::info!(
        target: "vchat::calls",
        from = %sender_onion,
        sequence = msg.sequence,
        "CallReject received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_call_end(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    let sender_onion = derive_sender_onion(msg)?;

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("call-ended", serde_json::json!({
            "sender_onion": sender_onion,
            "message_id": msg.message_id,
        })) {
            warn!("Failed to emit call-ended: {e}");
        }
    }

    tracing::info!(
        target: "vchat::calls",
        from = %sender_onion,
        sequence = msg.sequence,
        "CallEnd received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_typing_indicator(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    let payload: TypingPayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid TypingPayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;

    if payload.is_typing {
        if let Err(e) = crate::crypto::store::update_typing_indicator_received(&sender_onion).await {
            warn!("Failed to store typing indicator: {e}");
        }
    }

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("typing-indicator", serde_json::json!({
            "sender_onion": sender_onion,
            "is_typing": payload.is_typing,
        })) {
            warn!("Failed to emit typing-indicator: {e}");
        }
    }

    tracing::debug!(
        target: "vchat::typing",
        from = %sender_onion,
        is_typing = payload.is_typing,
        sequence = msg.sequence,
        "TypingIndicator received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_heartbeat(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    let _payload: Option<HeartbeatPayload> = serde_json::from_slice(&msg.payload).ok();

    tracing::debug!(
        target: "vchat::heartbeat",
        from = %hex::encode(&msg.sender_pubkey),
        sequence = msg.sequence,
        "Heartbeat from {addr}"
    );

    let uptime = chrono::Utc::now().timestamp() as u64 - APP_START_TIME.load(Ordering::Relaxed);

    let heartbeat_response = HeartbeatPayload {
        relay_hint: None,
        uptime_secs: uptime,
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
    use crate::messaging::protocol::FileMetaPayload;

    let meta: FileMetaPayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid FileMetaPayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;

    let identity = crate::crypto::store::load_identity().await?
        .ok_or_else(|| anyhow::anyhow!("Identity not initialized"))?;

    crate::crypto::store::save_file_transfer(
        &meta.file_id,
        &sender_onion,
        &identity.onion_address,
        &meta.filename,
        Some(&meta.mime_type),
        Some(meta.size as i64),
        None,
        None,
        "receiving",
    ).await?;

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("file-transfer-started", serde_json::json!({
            "file_id": meta.file_id,
            "sender_onion": sender_onion,
            "filename": meta.filename,
            "mime_type": meta.mime_type,
            "size": meta.size,
            "chunks_total": meta.chunks_total,
            "sha256": meta.sha256,
        })) {
            warn!("Failed to emit file-transfer-started: {e}");
        }
    }

    tracing::info!(
        target: "vchat::files",
        from = %sender_onion,
        file_id = %meta.file_id,
        filename = %meta.filename,
        size = meta.size,
        "FileMeta received from {addr}"
    );

    crate::error::audit_log(
        "file_meta_received",
        &format!("file_id={}, from={}, file={}", meta.file_id, sender_onion, meta.filename),
    );

    build_ack_response(msg).await
}

async fn handle_file_chunk(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    use crate::messaging::protocol::FileChunkPayload;

    let chunk: FileChunkPayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid FileChunkPayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;

    tracing::debug!(
        target: "vchat::files",
        from = %sender_onion,
        file_id = %chunk.file_id,
        chunk_index = chunk.chunk_index,
        chunk_size = chunk.data.len(),
        sequence = msg.sequence,
        "FileChunk received from {addr}"
    );

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("file-chunk-received", serde_json::json!({
            "file_id": chunk.file_id,
            "chunk_index": chunk.chunk_index,
            "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &chunk.data),
            "sender_onion": sender_onion,
        })) {
            warn!("Failed to emit file-chunk-received: {e}");
        }
    }

    build_ack_response(msg).await
}

async fn handle_reaction(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    use crate::messaging::protocol::ReactionPayload;

    let payload: ReactionPayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid ReactionPayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;
    let reaction_id = msg.message_id.clone();

    crate::crypto::store::save_reaction(
        &reaction_id,
        &payload.message_id,
        &sender_onion,
        &payload.emoji,
    ).await?;

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("new-reaction", serde_json::json!({
            "id": reaction_id,
            "message_id": payload.message_id,
            "sender": sender_onion,
            "emoji": payload.emoji,
        })) {
            warn!("Failed to emit new-reaction: {e}");
        }
    }

    tracing::info!(
        target: "vchat::reactions",
        from = %sender_onion,
        message_id = %payload.message_id,
        emoji = %payload.emoji,
        "Reaction received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_delivery_receipt(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    use crate::messaging::protocol::DeliveryReceiptPayload;

    let payload: DeliveryReceiptPayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid DeliveryReceiptPayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;

    for message_id in &payload.message_ids {
        if let Err(e) = crate::crypto::store::mark_message_delivered(message_id).await {
            warn!("Failed to mark message {message_id} delivered: {e}");
        }
    }

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("delivery-receipt", serde_json::json!({
            "sender_onion": sender_onion,
            "message_ids": payload.message_ids,
        })) {
            warn!("Failed to emit delivery-receipt: {e}");
        }
    }

    tracing::debug!(
        target: "vchat::receipts",
        from = %sender_onion,
        count = payload.message_ids.len(),
        "DeliveryReceipt received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_read_receipt(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    use crate::messaging::protocol::ReadReceiptPayload;

    let payload: ReadReceiptPayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid ReadReceiptPayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;

    for message_id in &payload.message_ids {
        if let Err(e) = crate::crypto::store::mark_message_read(message_id).await {
            warn!("Failed to mark message {message_id} read: {e}");
        }
    }

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("read-receipt", serde_json::json!({
            "sender_onion": sender_onion,
            "message_ids": payload.message_ids,
        })) {
            warn!("Failed to emit read-receipt: {e}");
        }
    }

    tracing::debug!(
        target: "vchat::receipts",
        from = %sender_onion,
        count = payload.message_ids.len(),
        "ReadReceipt received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_group_create(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    use crate::messaging::protocol::GroupCreatePayload;

    let payload: GroupCreatePayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid GroupCreatePayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;

    crate::crypto::store::save_group(
        &payload.group_id,
        &payload.name,
        None,
        None,
        &sender_onion,
    ).await?;

    for member in &payload.members {
        crate::crypto::store::add_group_member(
            &payload.group_id,
            &member.onion_address,
            Some(&member.public_key),
            Some(&member.display_name),
            &member.role,
        ).await?;
    }

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("group-created", serde_json::json!({
            "group_id": payload.group_id,
            "name": payload.name,
            "created_by": sender_onion,
        })) {
            warn!("Failed to emit group-created: {e}");
        }
    }

    tracing::info!(
        target: "vchat::groups",
        group_id = %payload.group_id,
        name = %payload.name,
        from = %sender_onion,
        "GroupCreate received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_group_message(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    use crate::messaging::protocol::GroupMessagePayload;

    let payload: GroupMessagePayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid GroupMessagePayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;

    let group_msg_id = uuid::Uuid::new_v4().to_string();

    crate::crypto::store::save_group_message(
        &group_msg_id,
        &payload.group_id,
        &sender_onion,
        Some(&payload.content),
        None,
        msg.timestamp,
        "text",
        Some(msg.sequence as i64),
        payload.reply_to.as_deref(),
    ).await?;

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("new-group-message", serde_json::json!({
            "id": group_msg_id,
            "group_id": payload.group_id,
            "sender": sender_onion,
            "content": payload.content,
            "timestamp": msg.timestamp,
            "message_type": "text",
            "reply_to": payload.reply_to,
        })) {
            warn!("Failed to emit new-group-message: {e}");
        }
    }

    tracing::info!(
        target: "vchat::groups",
        group_id = %payload.group_id,
        from = %sender_onion,
        "GroupMessage received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_group_update(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    use crate::messaging::protocol::GroupUpdatePayload;

    let payload: GroupUpdatePayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| anyhow::anyhow!("Invalid GroupUpdatePayload: {e}"))?;

    let sender_onion = derive_sender_onion(msg)?;

    if payload.update_type == "member_added" {
        if let Some(onion) = payload.data.get("onion_address").and_then(|v| v.as_str()) {
            let pk = payload.data.get("public_key").and_then(|v| v.as_str()).unwrap_or("");
            let name = payload.data.get("display_name").and_then(|v| v.as_str()).unwrap_or("");
            let _ = crate::crypto::store::add_group_member(&payload.group_id, onion, Some(pk), Some(name), "member").await;
        }
    } else if payload.update_type == "member_removed" {
        if let Some(onion) = payload.data.get("onion_address").and_then(|v| v.as_str()) {
            let _ = crate::crypto::store::remove_group_member(&payload.group_id, onion).await;
        }
    }

    if let Some(app) = APP_HANDLE.get() {
        if let Err(e) = app.emit("group-updated", serde_json::json!({
            "group_id": payload.group_id,
            "update_type": payload.update_type,
            "data": payload.data,
            "sender": sender_onion,
        })) {
            warn!("Failed to emit group-updated: {e}");
        }
    }

    tracing::info!(
        target: "vchat::groups",
        group_id = %payload.group_id,
        update_type = %payload.update_type,
        from = %sender_onion,
        "GroupUpdate received from {addr}"
    );

    build_ack_response(msg).await
}

async fn handle_default(msg: &WireMessage, addr: &std::net::SocketAddr) -> anyhow::Result<Vec<u8>> {
    warn!(
        "Unhandled message type {:?} from {addr}, seq={}",
        msg.msg_type, msg.sequence
    );

    build_ack_response(msg).await
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
