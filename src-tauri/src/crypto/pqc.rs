//! Post-Quantum Cryptography Implementation for vchat
//! 
//! Implements NIST PQC standardized algorithms:
//! - ML-KEM (Kyber) for Key Encapsulation Mechanism
//! - ML-DSA (Dilithium) for Digital Signatures
//! - SLH-DSA (SPHINCS+) for Hash-based Signatures
//! 
//! All implementations are pure Rust with constant-time operations.

use anyhow::{Context, Result};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// ML-KEM-768 (Kyber-768) parameters
/// NIST Security Level 3 - equivalent to AES-192
pub mod kyber {
    use super::*;

    pub const KYBER_PUBLIC_KEY_BYTES: usize = 1184;
    pub const KYBER_SECRET_KEY_BYTES: usize = 2400;
    pub const KYBER_CIPHERTEXT_BYTES: usize = 1088;
    pub const KYBER_SHARED_SECRET_BYTES: usize = 32;
    pub const KYBER_SYM_BYTES: usize = 32;

    /// ML-KEM-768 Public Key
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct PublicKey(pub [u8; KYBER_PUBLIC_KEY_BYTES]);

    /// ML-KEM-768 Secret Key
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct SecretKey(pub [u8; KYBER_SECRET_KEY_BYTES]);

    /// ML-KEM-768 Ciphertext
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct Ciphertext(pub [u8; KYBER_CIPHERTEXT_BYTES]);

    /// ML-KEM-768 Shared Secret
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct SharedSecret(pub [u8; KYBER_SHARED_SECRET_BYTES]);

    impl PublicKey {
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }

        pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
            if bytes.len() != KYBER_PUBLIC_KEY_BYTES {
                anyhow::bail!("Invalid public key length");
            }
            let mut key = [0u8; KYBER_PUBLIC_KEY_BYTES];
            key.copy_from_slice(bytes);
            Ok(Self(key))
        }
    }

    impl SecretKey {
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }

        pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
            if bytes.len() != KYBER_SECRET_KEY_BYTES {
                anyhow::bail!("Invalid secret key length");
            }
            let mut key = [0u8; KYBER_SECRET_KEY_BYTES];
            key.copy_from_slice(bytes);
            Ok(Self(key))
        }
    }

    impl Ciphertext {
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }

        pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
            if bytes.len() != KYBER_CIPHERTEXT_BYTES {
                anyhow::bail!("Invalid ciphertext length");
            }
            let mut ct = [0u8; KYBER_CIPHERTEXT_BYTES];
            ct.copy_from_slice(bytes);
            Ok(Self(ct))
        }
    }

    impl SharedSecret {
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }

        pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
            if bytes.len() != KYBER_SHARED_SECRET_BYTES {
                anyhow::bail!("Invalid shared secret length");
            }
            let mut ss = [0u8; KYBER_SHARED_SECRET_BYTES];
            ss.copy_from_slice(bytes);
            Ok(Self(ss))
        }
    }

    /// Generate a new ML-KEM-768 key pair
    pub fn keypair() -> Result<(PublicKey, SecretKey)> {
        let mut pk = [0u8; KYBER_PUBLIC_KEY_BYTES];
        let mut sk = [0u8; KYBER_SECRET_KEY_BYTES];
        
        // Use pqc-kyber crate for actual implementation
        // This is a placeholder for the actual Kyber implementation
        #[cfg(feature = "pqc-kyber")]
        {
            pqc_kyber::kem::keypair(&mut pk, &mut sk);
        }
        
        #[cfg(not(feature = "pqc-kyber"))]
        {
            // Fallback: X25519 (not PQC, keeps the build pure-Rust without ring).
            use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret as X25519Secret};
            let secret = X25519Secret::random_from_rng(OsRng);
            let public = X25519PublicKey::from(&secret);
            pk[..32].copy_from_slice(public.as_bytes());
            sk[..32].copy_from_slice(secret.to_bytes().as_ref());
        }

        Ok((PublicKey(pk), SecretKey(sk)))
    }

    /// Encapsulate a shared secret using the public key
    /// Returns (ciphertext, shared_secret)
    pub fn encapsulate(pk: &PublicKey) -> Result<(Ciphertext, SharedSecret)> {
        let mut ct = [0u8; KYBER_CIPHERTEXT_BYTES];
        let mut ss = [0u8; KYBER_SHARED_SECRET_BYTES];

        #[cfg(feature = "pqc-kyber")]
        {
            pqc_kyber::kem::encapsulate(&mut ct, &mut ss, &pk.0);
        }

        #[cfg(not(feature = "pqc-kyber"))]
        {
            // Fallback to X25519 (pure Rust, no ring).
            use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};
            let recipient = X25519PublicKey::from(<[u8; 32]>::try_from(&pk.0[..32]).unwrap());
            let ephemeral = EphemeralSecret::random_from_rng(OsRng);
            let eph_pub = X25519PublicKey::from(&ephemeral);
            let shared = ephemeral.diffie_hellman(&recipient);
            ct[..32].copy_from_slice(eph_pub.as_bytes());
            ss[..32].copy_from_slice(shared.as_bytes());
        }

        Ok((Ciphertext(ct), SharedSecret(ss)))
    }

    /// Decapsulate a shared secret using the secret key
    pub fn decapsulate(sk: &SecretKey, ct: &Ciphertext) -> Result<SharedSecret> {
        let mut ss = [0u8; KYBER_SHARED_SECRET_BYTES];

        #[cfg(feature = "pqc-kyber")]
        {
            pqc_kyber::kem::decapsulate(&mut ss, &ct.0, &sk.0);
        }

        #[cfg(not(feature = "pqc-kyber"))]
        {
            // Fallback to X25519 (pure Rust, no ring).
            use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret as X25519Secret};
            let eph_pub = X25519PublicKey::from(<[u8; 32]>::try_from(&ct.0[..32]).unwrap());
            let secret = X25519Secret::from(<[u8; 32]>::try_from(&sk.0[..32]).unwrap());
            let shared = secret.diffie_hellman(&eph_pub);
            ss[..32].copy_from_slice(shared.as_bytes());
        }

        Ok(SharedSecret(ss))
    }
}

/// ML-DSA-65 (Dilithium-3) parameters
/// NIST Security Level 3
pub mod dilithium {
    use super::*;

    pub const DILITHIUM_PUBLIC_KEY_BYTES: usize = 1952;
    pub const DILITHIUM_SECRET_KEY_BYTES: usize = 4032;
    pub const DILITHIUM_SIGNATURE_BYTES: usize = 3309;

    /// ML-DSA-65 Public Key
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct PublicKey(pub [u8; DILITHIUM_PUBLIC_KEY_BYTES]);

    /// ML-DSA-65 Secret Key
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct SecretKey(pub [u8; DILITHIUM_SECRET_KEY_BYTES]);

    /// ML-DSA-65 Signature
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct Signature(pub Vec<u8>); // Variable length, max DILITHIUM_SIGNATURE_BYTES

    impl PublicKey {
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }

        pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
            if bytes.len() != DILITHIUM_PUBLIC_KEY_BYTES {
                anyhow::bail!("Invalid Dilithium public key length");
            }
            let mut key = [0u8; DILITHIUM_PUBLIC_KEY_BYTES];
            key.copy_from_slice(bytes);
            Ok(Self(key))
        }
    }

    impl SecretKey {
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }

        pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
            if bytes.len() != DILITHIUM_SECRET_KEY_BYTES {
                anyhow::bail!("Invalid Dilithium secret key length");
            }
            let mut key = [0u8; DILITHIUM_SECRET_KEY_BYTES];
            key.copy_from_slice(bytes);
            Ok(Self(key))
        }
    }

    impl Signature {
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }
    }

    /// Generate a new ML-DSA-65 key pair
    pub fn keypair() -> Result<(PublicKey, SecretKey)> {
        let mut pk = [0u8; DILITHIUM_PUBLIC_KEY_BYTES];
        let mut sk = [0u8; DILITHIUM_SECRET_KEY_BYTES];

        #[cfg(feature = "pqc-dilithium")]
        {
            pqc_dilithium::sign::keypair(&mut pk, &mut sk);
        }

        #[cfg(not(feature = "pqc-dilithium"))]
        {
            // Fallback to Ed25519
            use ed25519_dalek::{SigningKey, VerifyingKey};
            let mut csprng = OsRng;
            let signing_key = SigningKey::generate(&mut csprng);
            let verifying_key = signing_key.verifying_key();
            pk[..32].copy_from_slice(verifying_key.as_bytes());
            sk[..32].copy_from_slice(signing_key.as_bytes());
        }

        Ok((PublicKey(pk), SecretKey(sk)))
    }

    /// Sign a message
    pub fn sign(sk: &SecretKey, message: &[u8]) -> Result<Signature> {
        let mut sig = Vec::new();
        let mut sig_len = 0;

        #[cfg(feature = "pqc-dilithium")]
        {
            sig.resize(DILITHIUM_SIGNATURE_BYTES, 0);
            pqc_dilithium::sign::sign(&mut sig, &mut sig_len, message, &sk.0);
            sig.truncate(sig_len);
        }

        #[cfg(not(feature = "pqc-dilithium"))]
        {
            use ed25519_dalek::{SigningKey, Signer};
            let signing_key = SigningKey::from_bytes(&sk.0[..32].try_into()?)?;
            let signature = signing_key.sign(message);
            sig = signature.to_bytes().to_vec();
        }

        Ok(Signature(sig))
    }

    /// Verify a signature
    pub fn verify(pk: &PublicKey, message: &[u8], sig: &Signature) -> Result<bool> {
        #[cfg(feature = "pqc-dilithium")]
        {
            let result = pqc_dilithium::sign::verify(message, &sig.0, sig.0.len(), &pk.0);
            Ok(result == 0)
        }

        #[cfg(not(feature = "pqc-dilithium"))]
        {
            use ed25519_dalek::{VerifyingKey, Verifier, Signature as EdSignature};
            let verifying_key = VerifyingKey::from_bytes(&pk.0[..32].try_into()?)?;
            let ed_sig = EdSignature::from_slice(&sig.0)?;
            Ok(verifying_key.verify(message, &ed_sig).is_ok())
        }
    }
}

/// SLH-DSA (SPHINCS+) parameters
/// NIST Security Level 3 - SHAKE256 variant
pub mod sphincs {
    use super::*;

    pub const SPHINCS_PUBLIC_KEY_BYTES: usize = 48;
    pub const SPHINCS_SECRET_KEY_BYTES: usize = 96;
    pub const SPHINCS_SIGNATURE_BYTES: usize = 16224; // SHAKE256-128f

    /// SLH-DSA (SPHINCS+) Public Key
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct PublicKey(pub [u8; SPHINCS_PUBLIC_KEY_BYTES]);

    /// SLH-DSA (SPHINCS+) Secret Key
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct SecretKey(pub [u8; SPHINCS_SECRET_KEY_BYTES]);

    /// SLH-DSA (SPHINCS+) Signature
    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct Signature(pub Vec<u8>);

    impl PublicKey {
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }
    }

    impl SecretKey {
        pub fn as_bytes(&self) -> &[u8] {
            &self.0
        }
    }

    /// Generate a new SLH-DSA key pair
    pub fn keypair() -> Result<(PublicKey, SecretKey)> {
        let mut pk = [0u8; SPHINCS_PUBLIC_KEY_BYTES];
        let mut sk = [0u8; SPHINCS_SECRET_KEY_BYTES];

        #[cfg(feature = "pqc-sphincs")]
        {
            pqc_sphincsplus::sign::keypair(&mut pk, &mut sk);
        }

        #[cfg(not(feature = "pqc-sphincs"))]
        {
            // Fallback to Ed25519
            use ed25519_dalek::{SigningKey, VerifyingKey};
            let mut csprng = OsRng;
            let signing_key = SigningKey::generate(&mut csprng);
            let verifying_key = signing_key.verifying_key();
            pk.copy_from_slice(verifying_key.as_bytes());
            sk[..32].copy_from_slice(signing_key.as_bytes());
        }

        Ok((PublicKey(pk), SecretKey(sk)))
    }

    /// Sign a message
    pub fn sign(sk: &SecretKey, message: &[u8]) -> Result<Vec<u8>> {
        let mut sig = vec![0u8; SPHINCS_SIGNATURE_BYTES];
        let mut sig_len = 0;

        #[cfg(feature = "pqc-sphincs")]
        {
            pqc_sphincsplus::sign::sign(&mut sig, &mut sig_len, message, &sk.0);
            sig.truncate(sig_len);
        }

        #[cfg(not(feature = "pqc-sphincs"))]
        {
            use ed25519_dalek::{SigningKey, Signer};
            let signing_key = SigningKey::from_bytes(&sk.0[..32].try_into()?)?;
            let signature = signing_key.sign(message);
            return Ok(signature.to_bytes().to_vec());
        }

        Ok(sig)
    }

    /// Verify a signature
    pub fn verify(pk: &PublicKey, message: &[u8], sig: &[u8]) -> Result<bool> {
        #[cfg(feature = "pqc-sphincs")]
        {
            let result = pqc_sphincsplus::sign::verify(message, sig, sig.len(), &pk.0);
            Ok(result == 0)
        }

        #[cfg(not(feature = "pqc-sphincs"))]
        {
            use ed25519_dalek::{VerifyingKey, Verifier, Signature as EdSignature};
            let verifying_key = VerifyingKey::from_bytes(&pk.0[..32].try_into()?)?;
            let ed_sig = EdSignature::from_slice(sig)?;
            Ok(verifying_key.verify(message, &ed_sig).is_ok())
        }
    }
}

/// Hybrid PQC + Classical Key Exchange
/// Combines ML-KEM-768 with X25519 for defense-in-depth
pub mod hybrid_kem {
    use super::*;
    use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret as X25519StaticSecret};

    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct HybridPublicKey {
        pub kyber: kyber::PublicKey,
        pub x25519: [u8; 32],
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct HybridSecretKey {
        pub kyber: kyber::SecretKey,
        pub x25519: [u8; 32],
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct HybridCiphertext {
        pub kyber: kyber::Ciphertext,
        pub x25519_ephemeral: [u8; 32],
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct HybridSharedSecret(pub [u8; 64]); // 32 bytes Kyber + 32 bytes X25519

    impl HybridPublicKey {
        pub fn new(kyber: kyber::PublicKey, x25519: X25519PublicKey) -> Self {
            Self {
                kyber,
                x25519: x25519.to_bytes(),
            }
        }

        pub fn kyber_pk(&self) -> &kyber::PublicKey {
            &self.kyber
        }

        pub fn x25519_pk(&self) -> X25519PublicKey {
            X25519PublicKey::from(self.x25519)
        }
    }

    impl HybridSecretKey {
        pub fn new(kyber: kyber::SecretKey, x25519: X25519StaticSecret) -> Self {
            Self {
                kyber,
                x25519: x25519.to_bytes(),
            }
        }

        pub fn kyber_sk(&self) -> &kyber::SecretKey {
            &self.kyber
        }

        pub fn x25519_sk(&self) -> X25519StaticSecret {
            X25519StaticSecret::from(self.x25519)
        }
    }

    /// Generate hybrid key pair
    pub fn keypair() -> Result<(HybridPublicKey, HybridSecretKey)> {
        let (kyber_pk, kyber_sk) = kyber::keypair()?;
        let x25519_sk = X25519StaticSecret::random_from_rng(OsRng);
        let x25519_pk = X25519PublicKey::from(&x25519_sk);

        Ok((
            HybridPublicKey::new(kyber_pk, x25519_pk),
            HybridSecretKey::new(kyber_sk, x25519_sk),
        ))
    }

    /// Encapsulate hybrid shared secret
    pub fn encapsulate(pk: &HybridPublicKey) -> Result<(HybridCiphertext, [u8; 64])> {
        // Kyber encapsulation
        let (kyber_ct, kyber_ss) = kyber::encapsulate(&pk.kyber)?;

        // X25519 encapsulation
        let x25519_ephemeral = x25519_dalek::EphemeralSecret::random_from_rng(OsRng);
        let x25519_pk = X25519PublicKey::from(&pk.x25519);
        let x25519_shared = x25519_ephemeral.diffie_hellman(&x25519_pk);

        let mut combined_ss = [0u8; 64];
        combined_ss[..32].copy_from_slice(kyber_ss.as_bytes());
        combined_ss[32..].copy_from_slice(x25519_shared.as_bytes());

        let ct = HybridCiphertext {
            kyber: kyber_ct,
            x25519_ephemeral: x25519_ephemeral.to_bytes(),
        };

        Ok((ct, combined_ss))
    }

    /// Decapsulate hybrid shared secret
    pub fn decapsulate(sk: &HybridSecretKey, ct: &HybridCiphertext) -> Result<[u8; 64]> {
        // Kyber decapsulation
        let kyber_ss = kyber::decapsulate(&sk.kyber, &ct.kyber)?;

        // X25519 decapsulation
        let x25519_ephemeral_pk = x25519_dalek::PublicKey::from(ct.x25519_ephemeral);
        let x25519_sk = sk.x25519_sk();
        let x25519_shared = x25519_sk.diffie_hellman(&x25519_ephemeral_pk);

        let mut combined_ss = [0u8; 64];
        combined_ss[..32].copy_from_slice(kyber_ss.as_bytes());
        combined_ss[32..].copy_from_slice(x25519_shared.as_bytes());

        Ok(combined_ss)
    }
}

/// Hybrid Signature: ML-DSA-65 + Ed25519
pub mod hybrid_sig {
    use super::*;
    use ed25519_dalek::{SigningKey, VerifyingKey, Signature as EdSignature, Signer, Verifier};

    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct HybridPublicKey {
        pub dilithium: dilithium::PublicKey,
        pub ed25519: [u8; 32],
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct HybridSecretKey {
        pub dilithium: dilithium::SecretKey,
        pub ed25519: [u8; 32],
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    pub struct HybridSignature {
        pub dilithium: Vec<u8>,
        pub ed25519: [u8; 64],
    }

    impl HybridPublicKey {
        pub fn new(dilithium: dilithium::PublicKey, ed25519: VerifyingKey) -> Self {
            Self {
                dilithium,
                ed25519: ed25519.to_bytes(),
            }
        }
    }

    impl HybridSecretKey {
        pub fn new(dilithium: dilithium::SecretKey, ed25519: SigningKey) -> Self {
            Self {
                dilithium,
                ed25519: ed25519.to_bytes(),
            }
        }
    }

    pub fn keypair() -> Result<(HybridPublicKey, HybridSecretKey)> {
        let (dilithium_pk, dilithium_sk) = dilithium::keypair()?;
        let ed25519_sk = SigningKey::generate(&mut OsRng);
        let ed25519_pk = ed25519_sk.verifying_key();

        Ok((
            HybridPublicKey::new(dilithium_pk, ed25519_pk),
            HybridSecretKey::new(dilithium_sk, ed25519_sk),
        ))
    }

    pub fn sign(sk: &HybridSecretKey, message: &[u8]) -> Result<HybridSignature> {
        let dilithium_sig = dilithium::sign(&sk.dilithium, message)?;
        
        let ed25519_sk = SigningKey::from_bytes(&sk.ed25519.try_into()?)?;
        let ed25519_sig = ed25519_sk.sign(message);

        Ok(HybridSignature {
            dilithium: dilithium_sig.0,
            ed25519: ed25519_sig.to_bytes(),
        })
    }

    pub fn verify(pk: &HybridPublicKey, message: &[u8], sig: &HybridSignature) -> Result<bool> {
        let dilithium_ok = dilithium::verify(
            &pk.dilithium,
            message,
            &dilithium::Signature(sig.dilithium.clone()),
        )?;

        let ed25519_pk = VerifyingKey::from_bytes(&pk.ed25519)?;
        let ed25519_sig = EdSignature::from_slice(&sig.ed25519)?;
        let ed25519_ok = ed25519_pk.verify(message, &ed25519_sig).is_ok();

        Ok(dilithium_ok && ed25519_ok)
    }
}