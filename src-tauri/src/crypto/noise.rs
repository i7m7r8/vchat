use anyhow::Result;
use snow::TransportState;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

pub struct NoiseSession {
    handshake: Option<snow::HandshakeState>,
    transport: Option<TransportState>,
    is_initiator: bool,
}

impl NoiseSession {
    pub fn new_initiator(static_secret: &StaticSecret) -> Result<Self> {
        let key_bytes = static_secret.to_bytes();
        let builder = snow::Builder::new(NOISE_PATTERN.parse()?)
            .local_private_key(&key_bytes)?;

        Ok(Self {
            handshake: Some(builder.build_initiator()?),
            transport: None,
            is_initiator: true,
        })
    }

    pub fn new_responder(static_secret: &StaticSecret) -> Result<Self> {
        let key_bytes = static_secret.to_bytes();
        let builder = snow::Builder::new(NOISE_PATTERN.parse()?)
            .local_private_key(&key_bytes)?;

        Ok(Self {
            handshake: Some(builder.build_responder()?),
            transport: None,
            is_initiator: false,
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

        anyhow::bail!("No active Noise session")
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

        anyhow::bail!("No active Noise session")
    }

    pub fn is_handshake_complete(&self) -> bool {
        self.handshake.is_none() && self.transport.is_some()
    }

    pub fn get_remote_static_key(&self) -> Option<Vec<u8>> {
        self.handshake
            .as_ref()
            .and_then(|hs| hs.get_remote_static().map(|k| k.to_vec()))
            .or(None)
    }

    pub fn is_initiator(&self) -> bool {
        self.is_initiator
    }
}

pub fn generate_static_keypair() -> Result<(StaticSecret, PublicKey)> {
    let secret = StaticSecret::random_from_rng(rand::thread_rng());
    let public = PublicKey::from(&secret);
    Ok((secret, public))
}

pub fn ephemeral_keypair() -> Result<(EphemeralSecret, PublicKey)> {
    let secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let public = PublicKey::from(&secret);
    Ok((secret, public))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noise_handshake() {
        let (initiator_secret, _initiator_public) = generate_static_keypair().unwrap();
        let (responder_secret, _responder_public) = generate_static_keypair().unwrap();

        let mut initiator = NoiseSession::new_initiator(&initiator_secret).unwrap();
        let mut responder = NoiseSession::new_responder(&responder_secret).unwrap();

        // Message 1: -> e
        let m1 = initiator.write_message(&[]).unwrap();
        assert!(!initiator.is_handshake_complete());
        let _ = responder.read_message(&m1).unwrap();

        // Message 2: <- e, ee, s, es
        let m2 = responder.write_message(&[]).unwrap();
        assert!(!responder.is_handshake_complete());
        let _ = initiator.read_message(&m2).unwrap();

        // Message 3: -> s, se
        let m3 = initiator.write_message(&[]).unwrap();
        assert!(initiator.is_handshake_complete());
        let _ = responder.read_message(&m3).unwrap();
        assert!(responder.is_handshake_complete());

        // Encrypted transport
        let encrypted = initiator.write_message(b"hello").unwrap();
        let decrypted = responder.read_message(&encrypted).unwrap();
        assert_eq!(decrypted, b"hello");

        let encrypted = responder.write_message(b"world").unwrap();
        let decrypted = initiator.read_message(&encrypted).unwrap();
        assert_eq!(decrypted, b"world");
    }
}
