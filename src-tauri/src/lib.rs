pub mod base32;
pub mod commands;
pub mod crypto;
pub mod error;
pub mod messaging;
pub mod tor;
pub mod webrtc;

// Core P2P modules (pure-Rust, no native deps)
pub mod dht;

// Optional heavy subsystems - off by default
#[cfg(feature = "jami-p2p")]
pub mod drt;
#[cfg(feature = "jami-p2p")]
pub mod file_transfer;
#[cfg(feature = "jami-p2p")]
pub mod identity;
#[cfg(feature = "jami-p2p")]
pub mod swarm;

use tracing_subscriber::EnvFilter;
use tauri::Manager;
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,vchat=debug")),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();

    tracing::info!("Starting Vchat v{}...", env!("CARGO_PKG_VERSION"));

    let app_state = commands::create_app_state();

    tauri::Builder::default()
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![
            commands::init_identity,
            commands::get_identity,
            commands::get_onion_address,
            commands::add_contact,
            commands::get_contacts,
            commands::get_contact,
            commands::delete_contact,
            commands::block_contact,
            commands::unblock_contact,
            commands::verify_contact,
            commands::send_message,
            commands::send_reply_message,
            commands::get_messages,
            commands::delete_message,
            commands::search_messages,
            commands::mark_messages_read,
            commands::set_disappearing_message,
            commands::add_reaction,
            commands::remove_reaction,
            commands::get_reactions,
            commands::send_typing_indicator,
            commands::get_typing_status,
            commands::create_group,
            commands::get_groups,
            commands::get_group,
            commands::add_group_member,
            commands::remove_group_member,
            commands::send_group_message,
            commands::get_group_messages,
            commands::get_group_members,
            commands::start_video_call,
            commands::start_audio_call,
            commands::answer_video_call,
            commands::end_video_call,
            commands::reject_call,
            commands::create_incoming_call,
            commands::send_voice_packet,
            commands::send_video_frame,
            commands::send_screen_frame,
            commands::gather_ice_candidates,
            commands::add_remote_ice_candidate,
            commands::connect_ice,
            commands::send_media_frame,
            commands::send_ice_candidate,
            commands::set_relay_config,
            commands::get_relay_config,
            commands::start_screen_share,
            commands::stop_screen_share,
            commands::toggle_audio_mute,
            commands::toggle_video,
            commands::get_call_history,
            commands::get_active_calls,
            commands::send_file,
            commands::get_file_transfers,
            commands::send_voice_note,
            commands::send_forward_message,
            commands::generate_qr_code,
            commands::scan_qr_code,
            commands::get_tor_status,
            commands::get_encryption_info,
            commands::delete_all_data,
            commands::get_settings,
            commands::update_settings,
            commands::set_media_key,
            commands::create_conference,
            commands::join_conference,
            commands::leave_conference,
            commands::get_conference,
            commands::forward_conference_media,
            commands::dht_start,
            commands::dht_bootstrap_from_peer,
            commands::dht_save_peer_cache,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let webrtc = app_state.webrtc.clone();

            // Initialize DB SYNCHRONOUSLY so all commands can use it immediately
            tauri::async_runtime::block_on(async {
                match app.path().app_data_dir() {
                    Ok(data_dir) => {
                        let db_path = data_dir.join("vchat.db");
                        tracing::info!("Using database at {}", db_path.display());
                        match crypto::store::init_db(&db_path).await {
                            Ok(()) => tracing::info!("Database initialized"),
                            Err(e) => {
                                tracing::error!("Database init failed: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to resolve app data directory: {e}");
                    }
                }
            });

            tauri::async_runtime::spawn(async move {
                crate::tor::hidden_service::set_app_handle(handle.clone());

                if let Err(e) = tor::init_tor(&handle).await {
                    tracing::error!("Tor init failed: {e}");
                }
            });

            tauri::async_runtime::spawn(async move {
                let cleanup_interval = std::time::Duration::from_secs(60);
                let typing_max_age = 300i64;

                loop {
                    tokio::time::sleep(cleanup_interval).await;

                    match crypto::store::cleanup_expired_messages(0).await {
                        Ok(n) if n > 0 => tracing::info!("Cleaned up {n} expired messages"),
                        Err(e) => tracing::error!("Message cleanup failed: {e}"),
                        _ => {}
                    }

                    match crypto::store::cleanup_typing_indicators(typing_max_age).await {
                        Ok(n) if n > 0 => tracing::info!("Cleaned up {n} stale typing indicators"),
                        Err(e) => tracing::error!("Typing indicator cleanup failed: {e}"),
                        _ => {}
                    }

                    if let Err(e) = webrtc::cleanup_old_calls(webrtc.clone()).await {
                        tracing::error!("Call cleanup failed: {e}");
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Vchat");
}
