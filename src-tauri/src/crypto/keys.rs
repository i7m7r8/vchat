use anyhow::Result;
use serde::{Deserialize, Serialize};

pub mod keys {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct KeyPair {
        pub public_key: Vec<u8>,
        pub private_key: Vec<u8>,
    }

    impl KeyPair {
        pub fn generate() -> Result<Self> {
            let static_secret = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
            let public_key = x25519_dalek::PublicKey::from(&static_secret);

            Ok(Self {
                public_key: public_key.as_bytes().to_vec(),
                private_key: static_secret.to_bytes().to_vec(),
            })
        }

        pub fn from_private_key(key: &[u8; 32]) -> Result<Self> {
            let static_secret = x25519_dalek::StaticSecret::from(*key);
            let public_key = x25519_dalek::PublicKey::from(&static_secret);

            Ok(Self {
                public_key: public_key.as_bytes().to_vec(),
                private_key: static_secret.to_bytes().to_vec(),
            })
        }

        pub fn public_key_hex(&self) -> String {
            hex::encode(&self.public_key)
        }

        pub fn private_key_hex(&self) -> String {
            hex::encode(&self.private_key)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_generate_keypair() {
            let kp = KeyPair::generate().unwrap();
            assert_eq!(kp.public_key.len(), 32);
            assert_eq!(kp.private_key.len(), 32);
        }

        #[test]
        fn test_from_private_key() {
            let kp1 = KeyPair::generate().unwrap();
            let secret_bytes: [u8; 32] = kp1.private_key.try_into().unwrap();
            let kp2 = KeyPair::from_private_key(&secret_bytes).unwrap();
            assert_eq!(kp1.public_key, kp2.public_key);
        }
    }
}
