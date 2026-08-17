use anyhow::Result;
use snow::{Builder, TransportState};
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

pub struct NoiseSession {
    transport: TransportState,
}

impl NoiseSession {
    pub fn new() -> Result<Self> {
        let builder = Builder::new(NOISE_PATTERN.parse()?);
        let keypair = builder.generate_keypair()?;

        Ok(Self {
            transport: builder
                .local_private_key(&keypair.private)
                .build_responder()?,
        })
    }

    pub fn initiator(static_secret: StaticSecret) -> Result<Self> {
        let builder = Builder::new(NOISE_PATTERN.parse()?)
            .local_private_key(&static_secret.to_bytes());

        Ok(Self {
            transport: builder.build_initiator()?,
        })
    }

    pub fn responder(static_secret: StaticSecret) -> Result<Self> {
        let builder = Builder::new(NOISE_PATTERN.parse()?)
            .local_private_key(&static_secret.to_bytes());

        Ok(Self {
            transport: builder.build_responder()?,
        })
    }

    pub fn write_message(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; 65535];
        let len = self.transport.write_message(payload, &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }

    pub fn read_message(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; 65535];
        let len = self.transport.read_message(data, &mut buf)?;
        buf.truncate(len);
        Ok(buf)
    }

    pub fn is_handshake_complete(&self) -> bool {
        self.transport.is_handshake_finished()
    }

    pub fn into_transport(self) -> Result<TransportState> {
        Ok(self.transport)
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
