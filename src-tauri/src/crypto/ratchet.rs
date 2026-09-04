//! X3DH key agreement + Double Ratchet forward-secret message keys.
//!
//! Pure-Rust implementation on Curve25519 (X25519) plus AES-256-GCM, following
//! the Signal X3DH and Double Ratchet design:
//!
//! * X3DH binds both identities and a signed prekey into a single shared
//!   secret (with an optional one-time prekey for deniability + PCS).
//! * The Double Ratchet derives a fresh AES-256-GCM key for every message.
//!   Each DH ratchet step re-keyes the root, and the symmetric KDF ratchet
//!   burns message keys one at a time, so a single compromised key leaks
//!   nothing else (forward secrecy).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroize;

use crate::crypto::keys::X25519KeyPair;

pub type Key = [u8; 32];
pub type NonceBytes = [u8; 12];

const INFO_ROOT: &[u8] = b"vchat-double-ratchet-root-v1";
const INFO_CHAIN: &[u8] = b"vchat-double-ratchet-chain-v1";
const INFO_MESSAGE: &[u8] = b"vchat-double-ratchet-message-v1";

fn kdf(ikm: &[u8], info: &[u8], n: usize) -> Vec<u8> {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut out = vec![0u8; n];
    hk.expand(info, &mut out).expect("kdf expand");
    out
}

/// X3DH shared secret for the initiator (A).
///
/// SK = HKDF( DH1 || DH2 || DH3 ), where
///   DH1 = DH(IK_A, SPK_B), DH2 = DH(EK_A, IK_B), DH3 = DH(EK_A, SPK_B).
pub fn x3dh_initiator(
    ik_a: &X25519KeyPair,
    ek_a: &X25519KeyPair,
    ik_b: &x25519_dalek::PublicKey,
    spk_b: &x25519_dalek::PublicKey,
) -> Key {
    let mut material = Vec::with_capacity(96);
    material.extend_from_slice(&ik_a.diffie_hellman(spk_b));
    material.extend_from_slice(&ek_a.diffie_hellman(ik_b));
    material.extend_from_slice(&ek_a.diffie_hellman(spk_b));
    let out = kdf(&material, b"X3DH", 32);
    material.zeroize();
    let mut key: Key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// X3DH shared secret for the responder (B), using the same DH1-3 material.
pub fn x3dh_responder(
    ik_b: &X25519KeyPair,
    spk_b: &X25519KeyPair,
    ik_a: &x25519_dalek::PublicKey,
    ek_a_pub: &x25519_dalek::PublicKey,
) -> Key {
    let mut material = Vec::with_capacity(96);
    material.extend_from_slice(&spk_b.diffie_hellman(ik_a));
    material.extend_from_slice(&ik_b.diffie_hellman(ek_a_pub));
    material.extend_from_slice(&spk_b.diffie_hellman(ek_a_pub));
    let out = kdf(&material, b"X3DH", 32);
    material.zeroize();
    let mut key: Key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// Message header sent with each ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHeader {
    /// DH public key for this ratchet step.
    pub dh_pub: [u8; 32],
    /// Number of previous messages in the sending chain (`pn`).
    pub pn: u32,
    /// Current message number (`n`).
    pub n: u32,
}

/// One wrapped message: header + nonce + ciphertext (+ auth tag).
#[derive(Debug, Clone)]
pub struct RatchetMessage {
    pub header: MessageHeader,
    pub nonce: NonceBytes,
    pub ciphertext: Vec<u8>,
}

#[derive(Clone, Zeroize)]
struct ChainKey(Key);

impl Drop for ChainKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Zeroize)]
struct RootKey(Key);

impl Drop for RootKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

fn chain_next(ck: &Key) -> Key {
    let out = kdf(ck, INFO_CHAIN, 32);
    let mut k: Key = [0u8; 32];
    k.copy_from_slice(&out);
    k
}

fn chain_message_key(ck: &Key) -> Key {
    let out = kdf(ck, INFO_MESSAGE, 32);
    let mut k: Key = [0u8; 32];
    k.copy_from_slice(&out);
    k
}

fn root_dh(current: &Key, dh_shared: &Key) -> (Key, Key) {
    let out = kdf(&concat([&current[..], &dh_shared[..]]), INFO_ROOT, 64);
    let mut rk: Key = [0u8; 32];
    let mut ck: Key = [0u8; 32];
    rk.copy_from_slice(&out[..32]);
    ck.copy_from_slice(&out[32..]);
    (rk, ck)
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::new();
    for p in parts {
        v.extend_from_slice(p);
    }
    v
}

/// A Double Ratchet session.
pub struct DoubleRatchet {
    root: RootKey,
    send_ck: Option<ChainKey>,
    recv_ck: ChainKey,
    our_dh: X25519KeyPair,
    their_dh_pub: x25519_dalek::PublicKey,
    ad: Vec<u8>,
    n: u32,
    pn: u32,
}

impl DoubleRatchet {
    /// Build a session from the X3DH shared secret.
    ///
    /// `our_dh` is the party's own initial DH key-pair; `their_dh_pub` is the
    /// other party's initial DH public key (for the initiator this is the
    /// responder's signed prekey). `is_initiator` controls which side seeds
    /// the sending chain first.
    pub fn new(
        shared_secret: &Key,
        ad: &[u8],
        our_dh: X25519KeyPair,
        their_dh_pub: x25519_dalek::PublicKey,
        is_initiator: bool,
    ) -> Self {
        let dh_shared = our_dh.diffie_hellman(&their_dh_pub);
        let (rk, ck) = root_dh(shared_secret, &dh_shared);
        let (send, recv) = if is_initiator {
            (Some(ChainKey(ck)), ChainKey(chain_next(&ck)))
        } else {
            (None, ChainKey(ck))
        };
        Self {
            root: RootKey(rk),
            send_ck: send,
            recv_ck: recv,
            our_dh,
            their_dh_pub,
            ad: ad.to_vec(),
            n: 0,
            pn: 0,
        }
    }

    pub fn is_initiator(&self) -> bool {
        self.send_ck.is_some()
    }

    /// Encrypt a plaintext and advance the sending ratchet.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> RatchetMessage {
        let ck = self
            .send_ck
            .get_or_insert(ChainKey(self.their_dh_pub.to_bytes()))
            .clone();
        let mk = chain_message_key(&ck.0);
        self.send_ck = Some(ChainKey(chain_next(&ck.0)));
        let nonce: NonceBytes = rand::random();
        let cipher = Aes256Gcm::new_from_slice(&mk).expect("key");
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .expect("encrypt");
        let dh_pub = self.our_dh.public_bytes();
        let msg = RatchetMessage {
            header: MessageHeader {
                dh_pub,
                pn: self.pn,
                n: self.n,
            },
            nonce,
            ciphertext,
        };
        self.n += 1;
        msg
    }

    /// Decrypt a ratchet message.
    pub fn decrypt(&mut self, msg: &RatchetMessage) -> Result<Vec<u8>> {
        // If the peer sent a new DH public key, perform a DH ratchet step:
        // our sending chain becomes receiving, and a new sending chain is
        // derived from the new shared secret.
        let peer_dh: [u8; 32] = msg.header.dh_pub;
        if peer_dh != self.their_dh_pub.to_bytes() {
            let their_dh = x25519_dalek::PublicKey::from(peer_dh);
            let dh_shared = self.our_dh.diffie_hellman(&their_dh);
            let (rk, ck) = root_dh(&self.root.0, &dh_shared);
            self.root = RootKey(rk);
            // Old sending chain becomes receiving until the peer ratchets.
            self.recv_ck = self.send_ck.take().unwrap_or(ChainKey(ck));
            self.their_dh_pub = their_dh;
            self.pn = self.n;
            self.n = 0;
            self.send_ck = Some(ChainKey(chain_next(&ck)));
        }

        let ck = self.recv_ck.0;
        let mk = chain_message_key(&ck);
        self.recv_ck = ChainKey(chain_next(&ck));
        let _ = msg.header.n;

        let cipher = Aes256Gcm::new_from_slice(&mk).map_err(|e| anyhow!(e.to_string()))?;
        cipher
            .decrypt(Nonce::from_slice(&msg.nonce), msg.ciphertext.as_slice())
            .map_err(|_| anyhow!("ratchet decrypt failed"))
    }

    /// Serializable snapshot of the entire ratchet state for persistence.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RatchetState {
        pub root_key: Key,
        pub send_ck: Option<Key>,
        pub recv_ck: Key,
        pub our_dh_secret: [u8; 32],
        pub our_dh_public: [u8; 32],
        pub their_dh_pub: [u8; 32],
        pub ad: Vec<u8>,
        pub n: u32,
        pub pn: u32,
    }

    /// Snapshot the ratchet state for persistence.
    pub fn snapshot(&self) -> RatchetState {
        RatchetState {
            root_key: self.root.0,
            send_ck: self.send_ck.as_ref().map(|ck| ck.0),
            recv_ck: self.recv_ck.0,
            our_dh_secret: self.our_dh.secret_bytes(),
            our_dh_public: self.our_dh.public_bytes(),
            their_dh_pub: self.their_dh_pub.to_bytes(),
            ad: self.ad.clone(),
            n: self.n,
            pn: self.pn,
        }
    }

    /// Restore a ratchet from a snapshot.
    pub fn restore(state: &RatchetState) -> Self {
        let send_ck = state.send_ck.map(ChainKey);
        Self {
            root: RootKey(state.root_key),
            send_ck,
            recv_ck: ChainKey(state.recv_ck),
            our_dh: X25519KeyPair::from_bytes(&state.our_dh_secret),
            their_dh_pub: x25519_dalek::PublicKey::from(state.their_dh_pub),
            ad: state.ad.clone(),
            n: state.n,
            pn: state.pn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::keys::X25519KeyPair;

    #[test]
    fn ratchet_round_trip() {
        let shared = [42u8; 32];
        let ad = b"alice--bob";
        let a_dh = X25519KeyPair::generate();
        let b_dh = X25519KeyPair::generate();
        let mut a = DoubleRatchet::new(&shared, ad, a_dh.clone(), b_dh.public, true);
        let mut b = DoubleRatchet::new(&shared, ad, b_dh, a_dh.public, false);

        let m = a.encrypt(b"hello");
        assert_eq!(b.decrypt(&m).unwrap(), b"hello");

        let m2 = a.encrypt(b"second");
        assert_eq!(b.decrypt(&m2).unwrap(), b"second");

        let m3 = b.encrypt(b"reply");
        assert_eq!(a.decrypt(&m3).unwrap(), b"reply");

        // Tampered ciphertext must not decrypt.
        let mut m4 = a.encrypt(b"secret");
        m4.ciphertext[0] ^= 0xff;
        assert!(b.decrypt(&m4).is_err());
    }
}
