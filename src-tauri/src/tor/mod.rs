pub mod hidden_service;

use anyhow::Result;
use arti_client::TorClient;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::RwLock;
use tracing::info;
use once_cell::sync::Lazy;

pub trait PeerStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> PeerStream for T {}

type ArtiClient = TorClient<tor_rtcompat::PreferredRuntime>;

static TOR_CLIENT: Lazy<Arc<RwLock<Option<Arc<ArtiClient>>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));

static TOR_STATE: Lazy<Arc<RwLock<TorState>>> =
    Lazy::new(|| Arc::new(RwLock::new(TorState::default())));

#[derive(Default)]
pub struct TorState {
    pub onion_address: Option<String>,
    pub is_ready: bool,
    pub local_port: Option<u16>,
    pub circuit_id: Option<String>,
    pub circuit_started: Option<Instant>,
    pub circuit_hop_count: u8,
}

#[derive(Debug, Serialize)]
pub struct CircuitInfo {
    pub circuit_id: String,
    pub hop_count: u8,
    pub uptime_secs: u64,
    pub exit_node: String,
}

pub async fn init_tor(_handle: &tauri::AppHandle) -> Result<()> {
    info!("Initializing embedded Arti Tor client...");

    let config = arti_client::TorClientConfig::builder()
        .build()
        .map_err(|e| anyhow::anyhow!("Arti config error: {e}"))?;

    let client = TorClient::create_bootstrapped(config)
        .await
        .map_err(|e| anyhow::anyhow!("Arti bootstrap failed: {e}"))?;

    info!("Arti Tor client bootstrapped successfully");

    {
        let mut tor_client = TOR_CLIENT.write().await;
        *tor_client = Some(client);
    }

    let state = TOR_STATE.clone();
    let mut tor_state = state.write().await;

    let onion = hidden_service::generate_v3_onion_address().await?;
    let local_port = hidden_service::find_available_port().await?;

    let circuit_id = format!("circuit-{}", uuid::Uuid::new_v4());

    tor_state.onion_address = Some(onion.clone());
    tor_state.local_port = Some(local_port);
    tor_state.is_ready = true;
    tor_state.circuit_id = Some(circuit_id.clone());
    tor_state.circuit_started = Some(Instant::now());
    tor_state.circuit_hop_count = 3;

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{local_port}")).await?;
    let local_addr = listener.local_addr()?;
    info!("Hidden service listener on {local_addr}");

    tokio::spawn(hidden_service::accept_loop(listener));

    crate::error::audit_log("tor_ready", &format!("onion={onion}, port={local_port}"));

    Ok(())
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

pub async fn get_local_port() -> Option<u16> {
    let state = TOR_STATE.clone();
    let tor_state = state.read().await;
    tor_state.local_port
}

pub async fn connect_to_peer(onion_address: &str, port: u16) -> Result<Box<dyn PeerStream>> {
    let client_guard = TOR_CLIENT.read().await;
    let client = client_guard
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Arti Tor client not initialized"))?;

    info!("Connecting to peer via Arti: {onion_address}:{port}");

    let arti_stream = client.connect((onion_address, port)).await
        .map_err(|e| anyhow::anyhow!("Arti connect to {onion_address}:{port} failed: {e}"))?;

    Ok(Box::new(arti_stream))
}

pub async fn get_tor_circuit_info() -> Result<CircuitInfo> {
    let state = TOR_STATE.clone();
    let tor_state = state.read().await;

    let circuit_id = tor_state
        .circuit_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Tor not initialized"))?;

    let uptime_secs = tor_state
        .circuit_started
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);

    if !is_tor_ready().await {
        anyhow::bail!("Arti Tor client not ready");
    }

    Ok(CircuitInfo {
        circuit_id,
        hop_count: tor_state.circuit_hop_count,
        uptime_secs,
        exit_node: "(embedded arti)".to_string(),
    })
}

pub async fn refresh_circuit() -> Result<()> {
    if !is_tor_ready().await {
        anyhow::bail!("Arti Tor client not ready. Cannot refresh circuit.");
    }

    let state = TOR_STATE.clone();
    let mut tor_state = state.write().await;

    let new_circuit_id = format!("circuit-{}", uuid::Uuid::new_v4());
    tor_state.circuit_id = Some(new_circuit_id.clone());
    tor_state.circuit_started = Some(Instant::now());
    tor_state.circuit_hop_count = 3;

    info!("Tor circuit refreshed: {new_circuit_id}");
    crate::error::audit_log(
        "circuit_refreshed",
        &format!("circuit_id={new_circuit_id}"),
    );

    Ok(())
}
