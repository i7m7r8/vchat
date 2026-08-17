use anyhow::Result;
use snow::TransportState;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

pub struct NoiseSession {
    handshake: Option<snow::HandshakeState>,
    transport: Option<TransportState>,
}

impl NoiseSession {
    pub fn new() -> Result<Self> {
        let builder = snow::Builder::new(NOISE_PATTERN.parse()?);
        Ok(Self {
            handshake: Some(builder.build_responder()?),
            transport: None,
        })
    }

    pub fn initiator(static_secret: StaticSecret) -> Result<Self> {
        let key_bytes = static_secret.to_bytes();
        let builder = snow::Builder::new(NOISE_PATTERN.parse()?)?
            .local_private_key(&key_bytes);
        Ok(Self {
            handshake: Some(builder.build_initiator()?),
            transport: None,
        })
    }

    pub fn responder(static_secret: StaticSecret) -> Result<Self> {
        let key_bytes = static_secret.to_bytes();
        let builder = snow::Builder::new(NOISE_PATTERN.parse()?)?
            .local_private_key(&key_bytes);
        Ok(Self {
            handshake: Some(builder.build_responder()?),
            transport: None,
        })
    }

    fn finish_handshake(&mut self) -> Result<()> {
        if let Some(hs) = self.handshake.take() {
            self.transport = Some(hs.into_transport_mode()?);
        }
        Ok(())
    }

    pub fn write_message(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; 65535];

        if let Some(ref mut hs) = self.handshake {
            let len = hs.write_message(payload, &mut buf)?;
            buf.truncate(len);
            if hs.is_handshake_finished() {
                self.finish_handshake()?;
            }
            return Ok(buf);
        }

        if let Some(ref mut transport) = self.transport {
            let len = transport.write_message(payload, &mut buf)?;
            buf.truncate(len);
            return Ok(buf);
        }

        anyhow::bail!("No active session")
    }

    pub fn read_message(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; 65535];

        if let Some(ref mut hs) = self.handshake {
            let len = hs.read_message(data, &mut buf)?;
            buf.truncate(len);
            if hs.is_handshake_finished() {
                self.finish_handshake()?;
            }
            return Ok(buf);
        }

        if let Some(ref mut transport) = self.transport {
            let len = transport.read_message(data, &mut buf)?;
            buf.truncate(len);
            return Ok(buf);
        }

        anyhow::bail!("No active session")
    }

    pub fn is_handshake_complete(&self) -> bool {
        self.handshake.is_none() && self.transport.is_some()
    }
}

pub fn generate_keypair() -> Result<(StaticSecret, PublicKey)> {
    let static_secret = StaticSecret::random_from_rng(rand::thread_rng());
    let public = PublicKey::from(&static_secret);
    Ok((static_secret, public))
}

pub fn ephemeral_keypair() -> Result<(EphemeralSecret, PublicKey)> {
    let secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let public = PublicKey::from(&secret);
    Ok((secret, public))
}
