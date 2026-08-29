use anyhow::{Context, Result};
use rustls::{
    client::ServerCertVerifier,
    server::ClientCertVerifier,
    Certificate, PrivateKey, RootCertStore,
    ServerConfig, ClientConfig,
    ServerConnection, ClientConnection,
    Connection,
};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use srtp::{
    Session,
    Policy,
    CryptoPolicy,
    Profile,
    SrtpError,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{mpsc, RwLock};
use tokio_rustls::{TlsConnector, TlsAcceptor, TlsStream};
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub struct MediaSession {
    pub session_id: String,
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub audio_send: Option<Arc<SrtpSession>>,
    pub audio_recv: Option<Arc<SrtpSession>>,
    pub video_send: Option<Arc<SrtpSession>>,
    pub video_recv: Option<Arc<SrtpSession>>,
    pub screen_send: Option<Arc<SrtpSession>>,
    pub screen_recv: Option<Arc<SrtpSession>>,
}

impl MediaSession {
    pub fn new(session_id: String, local_addr: SocketAddr, remote_addr: SocketAddr) -> Self {
        Self {
            session_id,
            local_addr,
            remote_addr,
            audio_send: None,
            audio_recv: None,
            video_send: None,
            video_recv: None,
            screen_send: None,
            screen_recv: None,
        }
    }
}

pub struct DtlsSrtpTransport {
    config: Arc<ServerConfig>,
    client_config: Arc<ClientConfig>,
    sessions: Arc<RwLock<HashMap<String, MediaSession>>>,
    cert_verifier: Arc<dyn CertVerifier>,
}

impl DtlsSrtpTransport {
    pub async fn new(cert_verifier: Arc<dyn CertVerifier>) -> Result<Self> {
        // Generate self-signed certificate for DTLS
        let cert_params = CertificateParams::new(vec!["vchat".to_string()])?;
        let key_pair = KeyPair::generate()?;
        let cert = cert_params.self_signed(&key_pair)?;

        let cert_der = cert.serialize_der()?;
        let key_der = key_pair.serialize_der();

        let certificate = Certificate(cert_der);
        let private_key = PrivateKey(key_der);

        // Server config (for incoming connections)
        let mut server_config = ServerConfig::builder()
            .with_safe_defaults()
            .with_client_cert_verifier(cert_verifier.clone())
            .with_single_cert(vec![certificate.clone()], private_key.clone())?;

        server_config.alpn_protocols = vec![b"srtp".to_vec()];

        // Client config (for outgoing connections)
        let mut roots = RootCertStore::empty();
        roots.add(&certificate)?;

        let mut client_config = ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(roots)
            .with_single_cert(vec![certificate], private_key)?;

        client_config.alpn_protocols = vec![b"srtp".to_vec()];

        Ok(Self {
            config: Arc::new(server_config),
            client_config: Arc::new(client_config),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            cert_verifier,
        })
    }

    pub async fn accept_tcp(&self, stream: TcpStream, session_id: String, local_addr: SocketAddr, remote_addr: SocketAddr) -> Result<DtlsStream> {
        let acceptor = TlsAcceptor::from(self.config.clone());
        let tls_stream = acceptor.accept(stream).await?;
        Ok(DtlsStream::Tcp(tls_stream))
    }

    pub async fn connect_tcp(&self, addr: SocketAddr, session_id: String, local_addr: SocketAddr, remote_addr: SocketAddr) -> Result<DtlsStream> {
        let connector = TlsConnector::from(self.client_config.clone());
        let stream = TcpStream::connect(addr).await?;
        let tls_stream = connector.connect("vchat".try_into()?, stream).await?;
        Ok(DtlsStream::Tcp(tls_stream))
    }

    pub async fn create_udp_session(&self, local_addr: SocketAddr, remote_addr: SocketAddr, session_id: String) -> Result<UdpSrtpSession> {
        let socket = Arc::new(UdpSocket::bind(local_addr).await?);
        socket.connect(remote_addr).await?;

        // Create SRTP session for this UDP socket
        let session = SrtpSession::new(socket, session_id.clone()).await?;

        Ok(UdpSrtpSession { socket, session })
    }

    pub async fn create_media_session(
        &self,
        session_id: String,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
    ) -> Result<MediaSession> {
        let session = MediaSession::new(session_id.clone(), local_addr, remote_addr);

        // Create audio SRTP sessions
        let audio_send = self.create_srtp_session(session_id.clone(), "audio_send").await?;
        let audio_recv = self.create_srtp_session(session_id.clone(), "audio_recv").await?;

        // Create video SRTP sessions
        let video_send = self.create_srtp_session(session_id.clone(), "video_send").await?;
        let video_recv = self.create_srtp_session(session_id.clone(), "video_recv").await?;

        let mut s = MediaSession::new(session_id, local_addr, remote_addr);
        s.audio_send = Some(Arc::new(audio_send));
        s.audio_recv = Some(Arc::new(audio_recv));
        s.video_send = Some(Arc::new(video_send));
        s.video_recv = Some(Arc::new(video_recv));

        self.sessions.write().await.insert(session_id, s.clone());
        Ok(s)
    }

    async fn create_srtp_session(&self, session_id: String, label: &str) -> Result<SrtpSession> {
        // Create DTLS connection for SRTP key exchange
        // This is simplified - in reality, you'd use DTLS-SRTP key derivation
        let policy = Policy {
            crypto_policy: CryptoPolicy::AES_CM_128_HMAC_SHA1_80,
            ssrc_type: srtp::SsrcType::AnyInbound,
            window_size: 1024,
            allow_repeat_tx: false,
            rtcp_mux: true,
        };

        // In real implementation, keys would be derived from DTLS handshake
        let key = vec![0u8; 16]; // Placeholder
        let salt = vec![0u8; 14];

        let session = srtp::Session::new(Profile::Aes128CmHmacSha1_80, &key, &salt, policy)?;

        Ok(SrtpSession {
            session,
            ssrc: rand::random(),
        })
    }
}

pub struct SrtpSession {
    session: srtp::Session,
    ssrc: u32,
}

impl SrtpSession {
    pub fn new(session: srtp::Session, ssrc: u32) -> Self {
        Self { session, ssrc }
    }

    pub async fn protect_rtp(&mut self, data: &mut [u8]) -> Result<Vec<u8>> {
        let protected = self.session.protect_rtp(data, self.ssrc)?;
        Ok(protected)
    }

    pub async fn unprotect_rtp(&mut self, data: &mut [u8]) -> Result<Vec<u8>> {
        let unprotected = self.session.unprotect_rtp(data, self.ssrc)?;
        Ok(unprotected)
    }

    pub async fn protect_rtcp(&mut self, data: &mut [u8]) -> Result<Vec<u8>> {
        let protected = self.session.protect_rtcp(data, self.ssrc)?;
        Ok(protected)
    }

    pub async fn unprotect_rtcp(&mut self, data: &mut [u8]) -> Result<Vec<u8>> {
        let unprotected = self.session.unprotect_rtcp(data, self.ssrc)?;
        Ok(unprotected)
    }
}

pub struct UdpSrtpSession {
    socket: Arc<UdpSocket>,
    session: SrtpSession,
}

impl UdpSrtpSession {
    pub async fn send_rtp(&self, data: &[u8]) -> Result<()> {
        let mut protected = self.session.protect_rtp(data).await?;
        self.socket.send(&protected).await?;
        Ok(())
    }

    pub async fn recv_rtp(&self, buf: &mut [u8]) -> Result<usize> {
        let n = self.socket.recv(buf).await?;
        let unprotected = self.session.unprotect_rtp(&mut buf[..n]).await?;
        Ok(unprotected.len())
    }
}

pub enum DtlsStream {
    Tcp(TlsStream<TcpStream>),
}

impl DtlsStream {
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self {
            DtlsStream::Tcp(stream) => stream.read(buf).await.map_err(Into::into),
        }
    }

    pub async fn write(&mut self, buf: &[u8]) -> Result<usize> {
        match self {
            DtlsStream::Tcp(stream) => stream.write(buf).await.map_err(Into::into),
        }
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        match self {
            DtlsStream::Tcp(stream) => stream.shutdown().await.map_err(Into::into),
        }
    }
}

pub trait CertVerifier: Send + Sync {
    fn verify_server_cert(
        &self,
        cert: &Certificate,
        intermediates: &[Certificate],
        server_name: &str,
    ) -> Result<()>;

    fn verify_client_cert(
        &self,
        cert: &Certificate,
        intermediates: &[Certificate],
    ) -> Result<()>;
}

pub struct VchatCertVerifier {
    known_identities: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl VchatCertVerifier {
    pub fn new(known_identities: Arc<RwLock<HashMap<String, Vec<u8>>>>) -> Self {
        Self { known_identities }
    }
}

impl CertVerifier for VchatCertVerifier {
    fn verify_server_cert(
        &self,
        cert: &Certificate,
        _intermediates: &[Certificate],
        server_name: &str,
    ) -> Result<()> {
        // Verify the certificate matches expected identity
        let identities = self.known_identities.blocking_read();
        if let Some(expected_pubkey) = identities.get(server_name) {
            // In real implementation, extract pubkey from cert and compare
            debug!("Verified server cert for {}", server_name);
            Ok(())
        } else {
            warn!("Unknown server identity: {}", server_name);
            Err(anyhow::anyhow!("Unknown identity"))
        }
    }

    fn verify_client_cert(
        &self,
        cert: &Certificate,
        _intermediates: &[Certificate],
    ) -> Result<()> {
        // Extract identity from certificate and verify
        debug!("Verifying client cert");
        Ok(())
    }
}

impl rustls::client::ServerCertVerifier for VchatCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &Certificate,
        intermediates: &[Certificate],
        server_name: &rustls::ServerName,
        _ocsp: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        self.verify_server_cert(end_entity, intermediates, server_name.as_ref())
            .map(|_| rustls::client::ServerCertVerified::assertion())
            .map_err(|_| rustls::Error::InvalidCertificate(rustls::CertificateError::Other("Verification failed".into())))
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &Certificate,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &Certificate,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA256,
        ]
    }
}

impl rustls::server::ClientCertVerifier for VchatCertVerifier {
    fn verify_client_cert(
        &self,
        end_entity: &Certificate,
        intermediates: &[Certificate],
        _now: std::time::SystemTime,
    ) -> Result<rustls::server::ClientCertVerified, rustls::Error> {
        self.verify_client_cert(end_entity, intermediates)
            .map(|_| rustls::server::ClientCertVerified::assertion())
            .map_err(|_| rustls::Error::InvalidCertificate(rustls::CertificateError::Other("Verification failed".into())))
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &Certificate,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::server::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::server::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &Certificate,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::server::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::server::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA256,
        ]
    }
}