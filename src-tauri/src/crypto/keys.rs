use anyhow::Result;
use ed25519_dalek::{SigningKey, VerifyingKey};
use zeroize::Zeroize;

#[derive(Zeroize)]
pub struct Ed25519KeyPair {
    signing_key_bytes: [u8; 32],
    pub verifying_key: VerifyingKey,
}

impl Ed25519KeyPair {
    pub fn generate() -> Result<Self> {
        let mut csprng = rand::thread_rng();
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();

        Ok(Self {
            signing_key_bytes: signing_key.to_bytes(),
            verifying_key,
        })
    }

    pub fn from_bytes(secret: &[u8; 32]) -> Result<Self> {
        let signing_key = SigningKey::from_bytes(secret);
        let verifying_key = signing_key.verifying_key();

        Ok(Self {
            signing_key_bytes: *secret,
            verifying_key,
        })
    }

    pub fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.signing_key_bytes)
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying_key.to_bytes()
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.verifying_key.to_bytes())
    }

    pub fn secret_key_hex(&self) -> String {
        hex::encode(self.signing_key_bytes)
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key().sign(message).to_bytes()
    }

    pub fn verify(&self, message: &[u8], signature: &[u8; 64]) -> bool {
        use ed25519_dalek::Signature;
        let sig = Signature::from_bytes(signature);
        self.verifying_key.verify_strict(message, &sig).is_ok()
    }
}

pub struct X25519KeyPair {
    secret: x25519_dalek::StaticSecret,
    pub public: x25519_dalek::PublicKey,
}

impl X25519KeyPair {
    pub fn generate() -> Self {
        let secret = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
        let public = x25519_dalek::PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn from_bytes(secret_bytes: &[u8; 32]) -> Self {
        let secret = x25519_dalek::StaticSecret::from(*secret_bytes);
        let public = x25519_dalek::PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    pub fn public_bytes(&self) -> [u8; 32] {
        *self.public.as_bytes()
    }

    pub fn diffie_hellman(&self, their_public: &x25519_dalek::PublicKey) -> [u8; 32] {
        self.secret.diffie_hellman(their_public).as_bytes().clone()
    }
}

pub fn derive_shared_key(
    our_secret: &x25519_dalek::StaticSecret,
    their_public: &x25519_dalek::PublicKey,
) -> [u8; 32] {
    our_secret.diffie_hellman(their_public).as_bytes().clone()
}

pub fn hkdf_derive(
    ikm: &[u8],
    salt: &[u8],
    info: &[u8],
    output_len: usize,
) -> Result<Vec<u8>> {
    use hkdf::Hkdf;
    use sha2::Sha256;

    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut output = vec![0u8; output_len];
    hk.expand(info, &mut output)
        .map_err(|e| anyhow::anyhow!("HKDF expand failed: {e}"))?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ed25519_keypair_generate_and_sign() {
        let kp = Ed25519KeyPair::generate().unwrap();
        let msg = b"test message";
        let sig = kp.sign(msg);
        assert!(kp.verify(msg, &sig));
        assert!(!kp.verify(b"wrong message", &sig));
    }

    #[test]
    fn test_x25519_dh() {
        let alice = X25519KeyPair::generate();
        let bob = X25519KeyPair::generate();
        let shared_a = alice.diffie_hellman(&bob.public);
        let shared_b = bob.diffie_hellman(&alice.public);
        assert_eq!(shared_a, shared_b);
    }

    #[test]
    fn test_hkdf_derive() {
        let key = hkdf_derive(b"input", b"salt", b"info", 32).unwrap();
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn test_ed25519_roundtrip() {
        let kp = Ed25519KeyPair::generate().unwrap();
        let secret = kp.signing_key_bytes;
        let kp2 = Ed25519KeyPair::from_bytes(&secret).unwrap();
        assert_eq!(kp.public_key_bytes(), kp2.public_key_bytes());
    }
}
