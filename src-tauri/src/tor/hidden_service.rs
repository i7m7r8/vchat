use sha2::{Digest, Sha512};
use std::net::TcpListener as StdTcpListener;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

pub fn generate_v3_onion_address() -> String {
    let secret = ed25519_dalek::SigningKey::from_bytes(&rand::random::<[u8; 32]>());
    let verifying = secret.verifying_key();
    pubkey_to_v3_onion(&verifying.to_bytes())
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

    let mut buf = vec![0u8; 4096];
    match stream.read(&mut buf).await {
        Ok(0) => {
            info!("Connection closed by {addr}");
        }
        Ok(n) => {
            info!("Received {n} bytes from {addr}");
            let response = b"VCHAT/1.0 OK\n";
            if let Err(e) = stream.write_all(response).await {
                warn!("Write error to {addr}: {e}");
            }
        }
        Err(e) => {
            warn!("Read error from {addr}: {e}");
        }
    }
}
