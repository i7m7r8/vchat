use anyhow::Result;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;
use tracing::{info, warn};

static TOR_STATE: once_cell::sync::Lazy<Arc<RwLock<TorState>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(TorState::default())));

#[derive(Default)]
struct TorState {
    onion_address: Option<String>,
    is_ready: bool,
}

pub async fn init_tor(_handle: &tauri::AppHandle) -> Result<()> {
    info!("Initializing Tor connection...");

    let state = TOR_STATE.clone();
    let mut tor_state = state.write().await;

    match init_tor_client().await {
        Ok(address) => {
            tor_state.onion_address = Some(address.clone());
            tor_state.is_ready = true;
            info!("Tor onion service ready: {}", address);
        }
        Err(e) => {
            warn!("Tor initialization failed: {}. Running in offline mode.", e);
            tor_state.is_ready = false;
        }
    }

    Ok(())
}

async fn init_tor_client() -> Result<String> {
    info!("Starting Arti Tor client...");

    let tor_config = tor_config::TorConfig::default();

    let runtime = tor_rtcompat::create_runtime()?;

    let client = arti_client::TorClient::builder()
        .config(tor_config)
        .create_runtime(runtime)?;

    let client = client.connect().await?;

    let onion_address = create_onion_service(&client).await?;

    Ok(onion_address)
}

async fn create_onion_service(
    client: &arti_client::TorClient<impl tor_rtcompat::Runtime>,
) -> Result<String> {
    use arti_client::config::onion_service::OnionServiceConfig;
    use arti_client::config::TorAddr;

    let config = OnionServiceConfig::builder()
        .address("vchat.onion".parse()?)
        .build()?;

    let (addr, _service) = client
        .launch_onion_service(config)
        .await?;

    Ok(addr.to_string())
}

pub async fn get_onion_address() -> Result<String> {
    let state = TOR_STATE.clone();
    let tor_state = state.read().await;

    tor_state
        .onion_address
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Tor not initialized"))
}

pub async fn is_tor_ready() -> bool {
    let state = TOR_STATE.clone();
    let tor_state = state.read().await;
    tor_state.is_ready
}

pub async fn connect_to_peer(onion_address: &str) -> Result<Vec<u8>> {
    let state = TOR_STATE.clone();
    let tor_state = state.read().await;

    if !tor_state.is_ready {
        anyhow::bail!("Tor not initialized");
    }

    let tor_config = tor_config::TorConfig::default();
    let runtime = tor_rtcompat::create_runtime()?;
    let client = arti_client::TorClient::builder()
        .config(tor_config)
        .create_runtime(runtime)?;

    let client = client.connect().await?;

    let addr: arti_client::config::TorAddr = onion_address.parse()?;
    let stream = client.connect(addr).await?;

    info!("Connected to peer: {}", onion_address);

    Ok(vec![])
}
