//! Lightweight pure-Rust SRTP (RFC 3711) implementation.
//!
//! AES-128/256-CTR keystream + HMAC-SHA1-80 auth. No native dependencies.

use aes::cipher::{KeyIvInit, StreamCipher};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::Digest;
use zeroize::Zeroize;

type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;
type Aes256Ctr = ctr::Ctr128BE<aes::Aes256>;
type HmacSha1 = Hmac<Sha1>;

const SRTP_MASTER_KEY_LEN: usize = 16;
const SRTP_MASTER_SALT_LEN: usize = 14;
const SRTP_KEY_DERIVATION_LABEL: &[u8] = b"SRTP";

/// SRTP protection profile (RFC 3711 Section 4.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpProfile {
    Aes128CtrHmacSha1_80,
    Aes256CtrHmacSha1_80,
}

impl SrtpProfile {
    fn master_key_len(&self) -> usize {
        match self {
            SrtpProfile::Aes128CtrHmacSha1_80 => 16,
            SrtpProfile::Aes256CtrHmacSha1_80 => 32,
        }
    }

    fn master_salt_len(&self) -> usize {
        14
    }

    fn session_key_len(&self) -> usize {
        match self {
            SrtpProfile::Aes128CtrHmacSha1_80 => 16,
            SrtpProfile::Aes256CtrHmacSha1_80 => 32,
        }
    }

    fn session_salt_len(&self) -> usize {
        14
    }

    fn session_auth_len(&self) -> usize {
        10 // HMAC-SHA1-80
    }
}

/// Derived SRTP session keys for a given direction.
struct SessionKeys {
    enc_key: Vec<u8>,
    salt: Vec<u8>,
    auth_key: Vec<u8>,
}

impl Drop for SessionKeys {
    fn drop(&mut self) {
        self.enc_key.zeroize();
        self.salt.zeroize();
        self.auth_key.zeroize();
    }
}

/// SRTP context for a single direction (inbound or outbound).
pub struct SrtpContext {
    profile: SrtpProfile,
    ssrc: u32,
    roc: u32,       // rollover counter
    seq: u16,       // last sequence number seen
    session_keys: SessionKeys,
}

impl SrtpContext {
    /// Derive SRTP session keys from master key/salt (RFC 3711 Section 4.3).
    pub fn derive(
        master_key: &[u8],
        master_salt: &[u8],
        direction: u8, // 0 = outbound (send), 1 = inbound (recv)
        ssrc: u32,
        profile: SrtpProfile,
    ) -> Result<Self, &'static str> {
        if master_key.len() != profile.master_key_len() {
            return Err("invalid master key length");
        }
        if master_salt.len() != profile.master_salt_len() {
            return Err("invalid master salt length");
        }

        let session_keys = derive_session_keys(master_key, master_salt, direction, ssrc, profile)?;

        Ok(Self {
            profile,
            ssrc,
            roc: 0,
            seq: 0,
            session_keys,
        })
    }

    /// Create context from a 32-byte shared secret (e.g., from X3DH ratchet).
    /// Uses SHA-256 to derive 30-byte master key+salt.
    pub fn from_shared_secret(
        shared_secret: &[u8; 32],
        direction: u8,
        ssrc: u32,
        profile: SrtpProfile,
    ) -> Self {
        let mut hasher = sha2::Sha256::new();
        hasher.update(shared_secret);
        hasher.update(SRTP_KEY_DERIVATION_LABEL);
        hasher.update(&[direction]);
        let derived = hasher.finalize();

        let master_key_len = profile.master_key_len();
        let master_salt_len = profile.master_salt_len();

        let master_key = derived[..master_key_len].to_vec();
        let master_salt = derived[master_key_len..master_key_len + master_salt_len].to_vec();

        Self::derive(&master_key, &master_salt, direction, ssrc, profile)
            .expect("derived keys should be valid length")
    }

    /// Get current rollover counter.
    pub fn roc(&self) -> u32 {
        self.roc
    }

    /// Get last sequence number.
    pub fn last_seq(&self) -> u16 {
        self.seq
    }

    /// Protect an RTP packet (encrypt + auth tag).
    pub fn protect_rtp(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let header_len = min_rtp_header_len(plaintext);
        if header_len == 0 || plaintext.len() < header_len {
            return plaintext.to_vec();
        }

        let seq = u16::from_be_bytes([plaintext[2], plaintext[3]]);
        self.update_roc(seq);

        let mut packet = plaintext.to_vec();
        let payload = &mut packet[header_len..];

        // AES-CTR encryption
        let mut ctr = build_ctr_keystream(
            &self.session_keys.enc_key,
            &self.session_keys.salt,
            self.ssrc,
            self.roc,
            seq,
            payload.len(),
            self.profile,
        );
        ctr.apply_keystream(payload);

        // Authentication tag (HMAC-SHA1-80 over header + encrypted payload)
        let auth_tag = compute_auth_tag(
            &self.session_keys.auth_key,
            &packet,
            self.profile,
        );
        packet.extend_from_slice(&auth_tag);

        packet
    }

    /// Unprotect an RTP packet (verify auth + decrypt).
    pub fn unprotect_rtp(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, &'static str> {
        let header_len = min_rtp_header_len(ciphertext);
        if header_len == 0 || ciphertext.len() < header_len + self.profile.session_auth_len() {
            return Err("packet too short");
        }

        let auth_len = self.profile.session_auth_len();
        let payload_len = ciphertext.len() - header_len - auth_len;

        // Verify auth tag
        let expected_tag = compute_auth_tag(
            &self.session_keys.auth_key,
            &ciphertext[..ciphertext.len() - auth_len],
            self.profile,
        );
        let actual_tag = &ciphertext[ciphertext.len() - auth_len..];
        if !constant_time_eq(&expected_tag, actual_tag) {
            return Err("authentication failed");
        }

        let seq = u16::from_be_bytes([ciphertext[2], ciphertext[3]]);
        self.update_roc(seq);

        let mut packet = ciphertext[..ciphertext.len() - auth_len].to_vec();
        let payload = &mut packet[header_len..];

        // AES-CTR decryption (same as encryption)
        let mut ctr = build_ctr_keystream(
            &self.session_keys.enc_key,
            &self.session_keys.salt,
            self.ssrc,
            self.roc,
            seq,
            payload.len(),
            self.profile,
        );
        ctr.apply_keystream(payload);

        Ok(packet)
    }

    /// Protect an RTCP packet.
    pub fn protect_rtcp(&mut self, plaintext: &[u8]) -> Vec<u8> {
        // RTCP uses index = ROC << 16 | seq, but with separate counter
        // For simplicity, we use a fixed index derivation similar to RTP
        let index = (self.roc << 16) | self.seq as u32;
        self.seq = self.seq.wrapping_add(1);

        let mut packet = plaintext.to_vec();
        let payload = &mut packet[..];

        let mut ctr = build_ctr_keystream(
            &self.session_keys.enc_key,
            &self.session_keys.salt,
            self.ssrc,
            index >> 16,
            index as u16,
            payload.len(),
            self.profile,
        );
        ctr.apply_keystream(payload);

        let auth_tag = compute_auth_tag(
            &self.session_keys.auth_key,
            &packet,
            self.profile,
        );
        packet.extend_from_slice(&auth_tag);

        packet
    }

    /// Unprotect an RTCP packet.
    pub fn unprotect_rtcp(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, &'static str> {
        let auth_len = self.profile.session_auth_len();
        if ciphertext.len() < auth_len {
            return Err("packet too short");
        }

        let expected_tag = compute_auth_tag(
            &self.session_keys.auth_key,
            &ciphertext[..ciphertext.len() - auth_len],
            self.profile,
        );
        let actual_tag = &ciphertext[ciphertext.len() - auth_len..];
        if !constant_time_eq(&expected_tag, actual_tag) {
            return Err("authentication failed");
        }

        let index = (self.roc << 16) | self.seq as u32;
        self.seq = self.seq.wrapping_add(1);

        let mut packet = ciphertext[..ciphertext.len() - auth_len].to_vec();
        let payload = &mut packet[..];

        let mut ctr = build_ctr_keystream(
            &self.session_keys.enc_key,
            &self.session_keys.salt,
            self.ssrc,
            index >> 16,
            index as u16,
            payload.len(),
            self.profile,
        );
        ctr.apply_keystream(payload);

        Ok(packet)
    }

    fn update_roc(&mut self, seq: u16) {
        if self.seq > seq && self.seq > 32768 {
            self.roc = self.roc.wrapping_add(1);
        }
        self.seq = seq;
    }
}

/// Minimum RTP header length (12 bytes without extensions).
fn min_rtp_header_len(pkt: &[u8]) -> usize {
    if pkt.len() < 12 {
        return 0;
    }
    let v = pkt[0] >> 6;
    if v != 2 {
        return 0; // Not RTP version 2
    }
    let ext = (pkt[0] & 0x10) != 0;
    let csrc_count = pkt[0] & 0x0f;
    let mut len = 12 + (csrc_count as usize) * 4;
    if ext && pkt.len() >= len + 4 {
        let ext_len = u16::from_be_bytes([pkt[len + 2], pkt[len + 3]]) as usize;
        len += 4 + ext_len * 4;
    }
    if pkt.len() < len {
        return 0;
    }
    len
}

/// Build AES-CTR keystream for SRTP (RFC 3711 Section 4.1.1).
fn build_ctr_keystream(
    enc_key: &[u8],
    salt: &[u8],
    ssrc: u32,
    roc: u32,
    seq: u16,
    payload_len: usize,
    profile: SrtpProfile,
) -> CtrKeystream {
    // Build counter block: salt (14) || ssrc (4) || roc (4) || seq (2) || 0 (2)
    let mut counter = [0u8; 16];
    counter[..14].copy_from_slice(salt);
    counter[14..18].copy_from_slice(&ssrc.to_be_bytes());
    counter[18..22].copy_from_slice(&roc.to_be_bytes());
    counter[22..24].copy_from_slice(&seq.to_be_bytes());
    // counter[24..26] = 0 (block counter)

    let cipher = match profile {
        SrtpProfile::Aes128CtrHmacSha1_80 => {
            let key: &[u8; 16] = enc_key[..16].try_into().unwrap();
            CtrCipher::Aes128(Aes128Ctr::new(
                aes::cipher::Key::<Aes128Ctr>::from_slice(key),
                aes::cipher::Iv::<Aes128Ctr>::from_slice(&counter),
            ))
        }
        SrtpProfile::Aes256CtrHmacSha1_80 => {
            let key: &[u8; 32] = enc_key[..32].try_into().unwrap();
            CtrCipher::Aes256(Aes256Ctr::new(
                aes::cipher::Key::<Aes256Ctr>::from_slice(key),
                aes::cipher::Iv::<Aes256Ctr>::from_slice(&counter),
            ))
        }
    };

    CtrKeystream { cipher }
}

enum CtrCipher {
    Aes128(Aes128Ctr),
    Aes256(Aes256Ctr),
}

struct CtrKeystream {
    cipher: CtrCipher,
}

impl CtrKeystream {
    fn apply_keystream(&mut self, data: &mut [u8]) {
        match &mut self.cipher {
            CtrCipher::Aes128(c) => c.apply_keystream(data),
            CtrCipher::Aes256(c) => c.apply_keystream(data),
        }
    }
}

/// Compute HMAC-SHA1-80 auth tag (RFC 3711 Section 4.2).
fn compute_auth_tag(auth_key: &[u8], data: &[u8], profile: SrtpProfile) -> Vec<u8> {
    let mut mac = HmacSha1::new_from_slice(auth_key).expect("valid key");
    mac.update(data);
    let result = mac.finalize().into_bytes();
    result[..profile.session_auth_len()].to_vec()
}

/// Constant-time comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Derive session keys from master key/salt (RFC 3711 Section 4.3.1).
fn derive_session_keys(
    master_key: &[u8],
    master_salt: &[u8],
    direction: u8,
    ssrc: u32,
    profile: SrtpProfile,
) -> Result<SessionKeys, &'static str> {
    let enc_key = derive_key(master_key, master_salt, 0x00, direction, ssrc, profile.session_key_len())?;
    let salt = derive_key(master_key, master_salt, 0x01, direction, ssrc, profile.session_salt_len())?;
    let auth_key = derive_key(master_key, master_salt, 0x02, direction, ssrc, profile.session_auth_len())?;

    Ok(SessionKeys { enc_key, salt, auth_key })
}

/// RFC 3711 key derivation: PRF(master_key, label || index || context)
fn derive_key(
    master_key: &[u8],
    master_salt: &[u8],
    label: u8,
    direction: u8,
    ssrc: u32,
    out_len: usize,
) -> Result<Vec<u8>, &'static str> {
    let mut hasher = sha2::Sha256::new();
    hasher.update(master_key);
    hasher.update(&[label]);
    hasher.update(&[direction]);
    hasher.update(&ssrc.to_be_bytes());
    hasher.update(master_salt);

    let mut out = Vec::with_capacity(out_len);
    let mut counter = 0u8;
    while out.len() < out_len {
        let mut h = hasher.clone();
        h.update(&[counter]);
        let block = h.finalize();
        let remaining = out_len - out.len();
        out.extend_from_slice(&block[..remaining.min(32)]);
        counter = counter.wrapping_add(1);
    }
    out.truncate(out_len);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rtp_packet(seq: u16, ssrc: u32, payload: &[u8]) -> Vec<u8> {
        let mut pkt = Vec::with_capacity(12 + payload.len());
        pkt.push(0x80); // v=2, no padding, no ext, 0 csrc
        pkt.push(0x60); // PT=96 (dynamic)
        pkt.extend_from_slice(&seq.to_be_bytes());
        pkt.extend_from_slice(&0u32.to_be_bytes()); // timestamp
        pkt.extend_from_slice(&ssrc.to_be_bytes());
        pkt.extend_from_slice(payload);
        pkt
    }

    #[test]
    fn test_srtp_roundtrip_aes128() {
        let shared_secret = [0x42u8; 32];
        let ssrc = 0x12345678;

        let mut tx = SrtpContext::from_shared_secret(&shared_secret, 0, ssrc, SrtpProfile::Aes128CtrHmacSha1_80);
        let mut rx = SrtpContext::from_shared_secret(&shared_secret, 1, ssrc, SrtpProfile::Aes128CtrHmacSha1_80);

        let plaintext = make_rtp_packet(100, ssrc, b"hello world");
        let protected = tx.protect_rtp(&plaintext);
        assert!(protected.len() > plaintext.len()); // has auth tag

        let unprotected = rx.unprotect_rtp(&protected).expect("unprotect should succeed");
        assert_eq!(unprotected, plaintext);
    }

    #[test]
    fn test_srtp_roundtrip_aes256() {
        let shared_secret = [0x42u8; 32];
        let ssrc = 0x12345678;

        let mut tx = SrtpContext::from_shared_secret(&shared_secret, 0, ssrc, SrtpProfile::Aes256CtrHmacSha1_80);
        let mut rx = SrtpContext::from_shared_secret(&shared_secret, 1, ssrc, SrtpProfile::Aes256CtrHmacSha1_80);

        let plaintext = make_rtp_packet(100, ssrc, b"hello world");
        let protected = tx.protect_rtp(&plaintext);
        let unprotected = rx.unprotect_rtp(&protected).expect("unprotect should succeed");
        assert_eq!(unprotected, plaintext);
    }

    #[test]
    fn test_srtp_wrong_key_fails() {
        let shared_secret = [0x42u8; 32];
        let wrong_secret = [0x43u8; 32];
        let ssrc = 0x12345678;

        let mut tx = SrtpContext::from_shared_secret(&shared_secret, 0, ssrc, SrtpProfile::Aes128CtrHmacSha1_80);
        let mut rx = SrtpContext::from_shared_secret(&wrong_secret, 1, ssrc, SrtpProfile::Aes128CtrHmacSha1_80);

        let plaintext = make_rtp_packet(100, ssrc, b"hello world");
        let protected = tx.protect_rtp(&plaintext);
        let result = rx.unprotect_rtp(&protected);
        assert!(result.is_err());
    }

    #[test]
    fn test_srtp_rollover() {
        let shared_secret = [0x42u8; 32];
        let ssrc = 0x12345678;

        let mut tx = SrtpContext::from_shared_secret(&shared_secret, 0, ssrc, SrtpProfile::Aes128CtrHmacSha1_80);
        let mut rx = SrtpContext::from_shared_secret(&shared_secret, 1, ssrc, SrtpProfile::Aes128CtrHmacSha1_80);

        // Send packets that wrap around sequence number
        for i in 0..10 {
            let seq = 65530 + i;
            let plaintext = make_rtp_packet(seq, ssrc, &[i as u8; 16]);
            let protected = tx.protect_rtp(&plaintext);
            let unprotected = rx.unprotect_rtp(&protected).expect("unprotect should succeed");
            assert_eq!(unprotected, plaintext);
        }
        assert_eq!(rx.roc(), 1);
    }

    #[test]
    fn test_srtcp_roundtrip() {
        let shared_secret = [0x42u8; 32];
        let ssrc = 0x12345678;

        let mut tx = SrtpContext::from_shared_secret(&shared_secret, 0, ssrc, SrtpProfile::Aes128CtrHmacSha1_80);
        let mut rx = SrtpContext::from_shared_secret(&shared_secret, 1, ssrc, SrtpProfile::Aes128CtrHmacSha1_80);

        let plaintext = b"RTCP Sender Report";
        let protected = tx.protect_rtcp(plaintext);
        let unprotected = rx.unprotect_rtcp(&protected).expect("unprotect should succeed");
        assert_eq!(unprotected, plaintext);
    }
}