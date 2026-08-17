pub mod base32;
pub mod commands;
pub mod crypto;
pub mod messaging;
pub mod tor;
pub mod webrtc;

use tracing_subscriber::EnvFilter;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting Vchat...");

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
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = tor::init_tor(&handle).await {
                    tracing::error!("Failed to initialize Tor: {}", e);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Vchat");
}
