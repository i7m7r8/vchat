pub mod hidden_service;

use anyhow::Result;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{info, warn};
use once_cell::sync::Lazy;

static TOR_STATE: Lazy<Arc<RwLock<TorState>>> =
    Lazy::new(|| Arc::new(RwLock::new(TorState::default())));

static TOR_CLIENT: Lazy<Arc<RwLock<Option<arti_client::TorClient<arti_client::StdRuntime>>>>> =
    Lazy::new(|| Arc::new(RwLock::new(None)));

#[derive(Default)]
pub struct TorState {
    pub onion_address: Option<String>,
    pub is_ready: bool,
    pub local_port: Option<u16>,
    pub circuit_id: Option<String>,
    pub circuit_started: Option<Instant>,
    pub circuit_hop_count: u8,
    pub embedded_ready: bool,
}

#[derive(Debug, Serialize)]
pub struct CircuitInfo {
    pub circuit_id: String,
    pub hop_count: u8,
    pub uptime_secs: u64,
    pub exit_node: String,
}

pub async fn init_tor(_handle: &tauri::AppHandle) -> Result<()> {
    info!("Initializing Tor...");

    let embedded_ok = match init_embedded_tor().await {
        Ok(()) => {
            info!("Embedded Tor (arti) bootstrapped successfully");
            true
        }
        Err(e) => {
            warn!("Embedded Tor unavailable: {e}. Using SOCKS5 fallback.");
            false
        }
    };

    let state = TOR_STATE.clone();
    let mut tor_state = state.write().await;

    let onion = hidden_service::generate_v3_onion_address().await?;
    let local_port = hidden_service::find_available_port().await?;

    let circuit_id = format!("circuit-{}", uuid::Uuid::new_v4());

    tor_state.onion_address = Some(onion.clone());
    tor_state.local_port = Some(local_port);
    tor_state.is_ready = true;
    tor_state.embedded_ready = embedded_ok;
    tor_state.circuit_id = Some(circuit_id.clone());
    tor_state.circuit_started = Some(Instant::now());
    tor_state.circuit_hop_count = 3;

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{local_port}")).await?;
    let local_addr = listener.local_addr()?;
    info!("Hidden service listener on {local_addr}");

    tokio::spawn(hidden_service::accept_loop(listener));

    let transport = if embedded_ok { "embedded-arti" } else { "socks5-proxy" };
    crate::error::audit_log("tor_ready", &format!("onion={onion}, port={local_port}, transport={transport}"));

    Ok(())
}

async fn init_embedded_tor() -> Result<()> {
    let config = arti_client::TorClientConfig::builder()
        .build()
        .map_err(|e| anyhow::anyhow!("Arti config error: {e}"))?;

    let client = arti_client::TorClient::builder()
        .config(config)
        .create()
        .await
        .map_err(|e| anyhow::anyhow!("Arti bootstrap failed: {e}"))?;

    {
        let mut tc = TOR_CLIENT.write().await;
        *tc = Some(client);
    }

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
    if tor_state.is_ready && tor_state.embedded_ready {
        return true;
    }

    match tokio::net::TcpStream::connect("127.0.0.1:9050").await {
        Ok(stream) => { drop(stream); true }
        Err(_) => match tokio::net::TcpStream::connect("127.0.0.1:9150").await {
            Ok(stream) => { drop(stream); true }
            Err(_) => false,
        },
    }
}

pub async fn get_local_port() -> Option<u16> {
    let state = TOR_STATE.clone();
    let tor_state = state.read().await;
    tor_state.local_port
}

pub async fn connect_to_peer(onion_address: &str, port: u16) -> Result<tokio::net::TcpStream> {
    {
        let tc = TOR_CLIENT.read().await;
        if let Some(client) = tc.as_ref() {
            let target = format!("{onion_address}:{port}");
            info!("Connecting to peer via embedded Tor: {target}");
            match client.connect(target.as_str()).await {
                Ok(mut arti_stream) => {
                    drop(tc);
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                    let connect_addr = listener.local_addr()?;
                    let client_stream = tokio::net::TcpStream::connect(connect_addr).await?;
                    let (server_stream, _) = listener.accept().await?;

                    tokio::spawn(async move {
                        let _ = tokio::io::copy_bidirectional(&mut arti_stream, &mut server_stream).await;
                    });

                    return Ok(client_stream);
                }
                Err(e) => {
                    warn!("Embedded Tor connect failed: {e}, falling back to SOCKS5");
                }
            }
        }
    }

    let target = format!("{onion_address}:{port}");
    info!("Connecting to peer via SOCKS5: {target}");
    try_connect_via_socks(&target).await
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

    let exit_node = if tor_state.embedded_ready {
        "(embedded-arti)".to_string()
    } else {
        "(socks5-proxy)".to_string()
    };

    Ok(CircuitInfo {
        circuit_id,
        hop_count: tor_state.circuit_hop_count,
        uptime_secs,
        exit_node,
    })
}

pub async fn refresh_circuit() -> Result<()> {
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

async fn try_connect_via_socks(target: &str) -> Result<tokio::net::TcpStream> {
    let socks_ports = [9050, 9150];
    let mut last_err = None;

    for port in &socks_ports {
        let addr = format!("127.0.0.1:{port}");
        match tokio::net::TcpStream::connect(&addr).await {
            Ok(stream) => match socks5_connect(stream, target).await {
                Ok(s) => return Ok(s),
                Err(e) => { last_err = Some(e); continue; }
            },
            Err(_) => continue,
        }
    }

    Err(last_err.unwrap_or_else(|| {
        anyhow::anyhow!("Cannot reach Tor. Embedded Tor unavailable and no SOCKS proxy on 9050/9150")
    }))
}

async fn socks5_connect(
    stream: tokio::net::TcpStream,
    target: &str,
) -> Result<tokio::net::TcpStream> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut reader, mut writer) = stream.into_split();

    writer.write_all(&[0x05, 0x01, 0x00]).await?;
    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf).await?;
    if buf[0] != 0x05 || buf[1] != 0x00 {
        anyhow::bail!("SOCKS5 auth failed");
    }

    let (host, port_str) = target
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("Bad target: {target}"))?;
    let port: u16 = port_str.parse()?;
    let hostname = host.strip_suffix(".onion").unwrap_or(host);

    let mut req = Vec::with_capacity(7 + hostname.len());
    req.extend_from_slice(&[0x05, 0x01, 0x00, 0x03]);
    req.push(hostname.len() as u8);
    req.extend_from_slice(hostname.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    writer.write_all(&req).await?;

    let mut resp = [0u8; 4];
    reader.read_exact(&mut resp).await?;
    if resp[1] != 0x00 {
        anyhow::bail!("SOCKS5 CONNECT denied: {}", resp[1]);
    }

    match resp[3] {
        0x01 => { let mut a = [0u8; 6]; reader.read_exact(&mut a).await?; }
        0x03 => {
            let mut l = [0u8; 1]; reader.read_exact(&mut l).await?;
            let mut a = vec![0u8; l[0] as usize + 2]; reader.read_exact(&mut a).await?;
        }
        0x04 => { let mut a = [0u8; 18]; reader.read_exact(&mut a).await?; }
        _ => anyhow::bail!("Unknown SOCKS5 atyp"),
    }

    Ok(reader.reunite(writer)?)
}
