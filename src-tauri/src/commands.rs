use serde::{Deserialize, Serialize};
use crate::crypto;
use crate::tor;
use crate::messaging;
use crate::webrtc;

#[derive(Debug, Serialize, Deserialize)]
pub struct Identity {
    pub public_key: String,
    pub onion_address: String,
    pub display_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub display_name: String,
    pub public_key: String,
    pub onion_address: String,
    pub added_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub content: String,
    pub timestamp: i64,
    pub encrypted: bool,
    pub message_type: MessageType,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum MessageType {
    Text,
    File,
    Media,
    System,
}

#[tauri::command]
pub async fn init_identity(display_name: String) -> Result<Identity, String> {
    let identity = crypto::generate_identity(&display_name)
        .await
        .map_err(|e| e.to_string())?;
    Ok(identity)
}

#[tauri::command]
pub async fn get_identity() -> Result<Option<Identity>, String> {
    crypto::load_identity()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_onion_address() -> Result<String, String> {
    tor::get_onion_address()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_message(
    recipient_onion: String,
    content: String,
    message_type: MessageType,
) -> Result<Message, String> {
    messaging::send_message(&recipient_onion, &content, message_type)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_video_call(recipient_onion: String) -> Result<String, String> {
    webrtc::start_video_call(&recipient_onion)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn answer_video_call(call_id: String) -> Result<(), String> {
    webrtc::answer_video_call(&call_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn end_video_call(call_id: String) -> Result<(), String> {
    webrtc::end_video_call(&call_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_screen_share(call_id: String) -> Result<(), String> {
    webrtc::start_screen_share(&call_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_screen_share(call_id: String) -> Result<(), String> {
    webrtc::stop_screen_share(&call_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_contact(
    display_name: String,
    public_key: String,
    onion_address: String,
) -> Result<Contact, String> {
    messaging::add_contact(&display_name, &public_key, &onion_address)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_contacts() -> Result<Vec<Contact>, String> {
    messaging::get_contacts()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_messages(contact_onion: String) -> Result<Vec<Message>, String> {
    messaging::get_messages(&contact_onion)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn generate_qr_code() -> Result<String, String> {
    crypto::generate_qr_code()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn scan_qr_code(qr_data: String) -> Result<Contact, String> {
    crypto::scan_qr_code(&qr_data)
        .await
        .map_err(|e| e.to_string())
}
