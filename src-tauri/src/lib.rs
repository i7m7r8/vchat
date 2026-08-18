pub mod base32;
pub mod commands;
pub mod crypto;
pub mod error;
pub mod messaging;
pub mod tor;
pub mod webrtc;

use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,vchat=debug")),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();

    tracing::info!("Starting Vchat v{}...", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::init_identity,
            commands::get_identity,
            commands::get_onion_address,
            commands::send_message,
            commands::start_video_call,
            commands::answer_video_call,
            commands::end_video_call,
            commands::start_screen_share,
            commands::stop_screen_share,
            commands::add_contact,
            commands::get_contacts,
            commands::get_messages,
            commands::generate_qr_code,
            commands::scan_qr_code,
            commands::get_tor_status,
            commands::delete_all_data,
            commands::get_encryption_info,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crypto::store::init_db().await {
                    tracing::error!("Database init failed: {e}");
                    return;
                }
                tracing::info!("Database initialized");

                if let Err(e) = tor::init_tor(&handle).await {
                    tracing::error!("Tor init failed: {e}");
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Vchat");
}
