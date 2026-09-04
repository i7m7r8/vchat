//! Persistent per-peer Double Ratchet sessions for message encryption.
//!
//! Wraps [`crate::crypto::ratchet::DoubleRatchet`] so the full ratchet state
//! (root key, both chain keys, our DH key, the peer's last DH public key and
//! counters) can be serialized and restored from the `sessions` SQLite table,
//! giving true forward secrecy for the messaging path.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::crypto::keys::X25519KeyPair;
use crate::crypto::ratchet::{DoubleRatchet, Key, MessageHeader, NonceBytes, RatchetState};

/// Serializable snapshot of a live ratchet session (re-exported from ratchet).
pub type SessionState = RatchetState;

/// A resumable Double Ratchet session bound to one peer.
pub struct Session {
    ratchet: DoubleRatchet,
}

impl Session {
    /// Create a fresh session from an X3DH shared secret.
    pub fn new(
        shared_secret: &Key,
        ad: &[u8],
        our_dh: X25519KeyPair,
        their_dh_pub: &x25519_dalek::PublicKey,
        is_initiator: bool,
    ) -> Self {
        let ratchet = DoubleRatchet::new(shared_secret, ad, our_dh, *their_dh_pub, is_initiator);
        Self { ratchet }
    }

    /// Encrypt a plaintext message for the peer.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let m = self.ratchet.encrypt(plaintext);
        let mut buf = Vec::new();
        buf.extend_from_slice(&m.header.dh_pub);
        buf.extend_from_slice(&m.header.pn.to_le_bytes());
        buf.extend_from_slice(&m.header.n.to_le_bytes());
        buf.extend_from_slice(&m.nonce);
        buf.extend_from_slice(&(m.ciphertext.len() as u32).to_le_bytes());
        buf.extend_from_slice(&m.ciphertext);
        Ok(buf)
    }

    /// Decrypt a message received from the peer.
    pub fn decrypt(&mut self, wire: &[u8]) -> Result<Vec<u8>> {
        if wire.len() < 32 + 4 + 4 + 12 + 4 {
            Err(anyhow!("ratchet message too short"))?;
        }
        let mut dh_pub = [0u8; 32];
        dh_pub.copy_from_slice(&wire[0..32]);
        let pn = u32::from_le_bytes(wire[32..36].try_into().unwrap());
        let n = u32::from_le_bytes(wire[36..40].try_into().unwrap());
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&wire[40..52]);
        let len = u32::from_le_bytes(wire[52..56].try_into().unwrap()) as usize;
        let ciphertext = wire[56..56 + len].to_vec();
        self.ratchet
            .decrypt(&crate::crypto::ratchet::RatchetMessage {
                header: MessageHeader { dh_pub, pn, n },
                nonce,
                ciphertext,
            })
    }

    /// Snapshot for persistence.
    pub fn snapshot(&self) -> Result<SessionState> {
        // DoubleRatchet fields are private; expose a snapshot via a dedicated
        // method on DoubleRatchet for serialization.
        self.ratchet.snapshot()
    }

    /// Restore from a snapshot.
    pub fn restore(state: &SessionState) -> Result<Self> {
        let ratchet = DoubleRatchet::restore(state)?;
        Ok(Self { ratchet })
    }

    /// Whether this session is the initiator.
    pub fn is_initiator(&self) -> Result<bool> {
        Ok(self.ratchet.is_initiator())
    }
}

/// Serialize a session snapshot to a string for the `sessions` table.
pub fn serialize_state(state: &SessionState) -> Result<String> {
    Ok(serde_json::to_string(state)?)
}

/// Parse a session snapshot from the `sessions` table.
pub fn deserialize_state(s: &str) -> Result<SessionState> {
    Ok(serde_json::from_str(s)?)
}

/// Helper zeroization for a key in-memory.
pub fn wipe(mut k: Key) {
    k.zeroize();
}

/// Build the associated-data binding for two peers (canonical ordering).
pub fn associated_data(a_onion: &str, b_onion: &str) -> Vec<u8> {
    let mut ad = Vec::with_capacity(a_onion.len() + b_onion.len() + 1);
    if a_onion <= b_onion {
        ad.extend_from_slice(a_onion.as_bytes());
        ad.push(0);
        ad.extend_from_slice(b_onion.as_bytes());
    } else {
        ad.extend_from_slice(b_onion.as_bytes());
        ad.push(0);
        ad.extend_from_slice(a_onion.as_bytes());
    }
    ad
}

/// Convenience: export our latest DH public key for ratchet initiation.
pub fn our_ratchet_public(our_dh: &X25519KeyPair) -> [u8; 32] {
    our_dh.public_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::keys::X25519KeyPair;

    #[test]
    fn session_state_round_trip() {
        let shared = [9u8; 32];
        let ad = associated_data("alice.onion", "bob.onion");
        let a_dh = X25519KeyPair::generate();
        let b_dh = X25519KeyPair::generate();

        let mut a = Session::new(&shared, &ad, a_dh.clone(), &b_dh.public, true);
        let mut b = Session::new(&shared, &ad, b_dh, &a_dh.public, false);

        let ct = a.encrypt(b"ping").unwrap();
        assert_eq!(b.decrypt(&ct).unwrap(), b"ping");

        // Persist and restore the initiator's session, then continue.
        let a_state = a.snapshot().unwrap();
        let s = serialize_state(&a_state).unwrap();
        let restored = deserialize_state(&s).unwrap();
        let mut a2 = Session::restore(&restored).unwrap();

        let ct2 = b.encrypt(b"pong").unwrap();
        assert_eq!(a2.decrypt(&ct2).unwrap(), b"pong");
    }
}
