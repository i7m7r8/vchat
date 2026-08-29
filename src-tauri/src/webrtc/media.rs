//! Media capture pipeline for calls (audio / video / screen share).
//!
//! This module is the software media path that feeds the per-call UDP + SRTP
//! transport in [`super`]. It owns the codec framing and keying so that the
//! transport only deals with opaque, already-protected payloads.
//!
//! It deliberately keeps the codec layer pluggable: the default build ships a
//! lightweight pure-Rust pipeline (raw PCM + uncompressed frames are not
//! practical over a network, so real codec plug-ins can be swapped in behind
//! [`Encoder`] / [`Decoder`] without changing the transport).

use serde::{Deserialize, Serialize};

use super::{MediaFrameKind, SrtpContext};

/// The encoded sampling rate / dimensions used by the default pipeline.
pub const AUDIO_CLOCK_RATE: u32 = 48000;

/// A single decoded/renderable media sample ready for a codec or renderer.
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    /// Which track this frame belongs to.
    pub kind: MediaFrameKind,
    /// RTP timestamp (monotonic in the track's clock rate).
    pub rtp_timestamp: u32,
    /// Synchronisation source id (SSRC) for the track.
    pub ssrc: u32,
    /// Media payload:
    ///   - Voice -> mono PCM (i16 LE) or encoded opus bytes
    ///   - Video / Screen -> one encoded frame of compressed bytes
    pub data: Vec<u8>,
}

impl CapturedFrame {
    pub fn new(kind: MediaFrameKind, rtp_timestamp: u32, ssrc: u32, data: Vec<u8>) -> Self {
        Self {
            kind,
            rtp_timestamp,
            ssrc,
            data,
        }
    }
}

/// Payload header placed in front of every captured frame so the far end can
/// demultiplex tracks and validate size limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrameHeader {
    pub kind: u8,
    pub track: u8,
    pub ctx_seq: u16,
    pub rtp_timestamp: u32,
    pub payload_len: u32,
}

impl FrameHeader {
    pub fn from(u8kind: u8, track: u8, ctx_seq: u16, rtp_timestamp: u32, payload_len: usize) -> Self {
        Self {
            kind: u8kind,
            track,
            ctx_seq,
            rtp_timestamp,
            payload_len: payload_len as u32,
        }
    }
}

/// How big a single protected media datagram may be before it must be split.
/// The default UDP transport uses 64 KiB datagrams; we keep well under that
/// after SRTP expansion and the ICE chunk header.
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024;

/// A single track's encoder/session keying inside a pipeline.
#[derive(Debug)]
pub struct TrackSession {
    pub ssrc: u32,
    pub srtp: SrtpContext,
    /// Sequence number used both as the SRTP rolling counter and frame order.
    pub seq: u16,
    /// Monotonic RTP timestamp for the track.
    pub rtp_timestamp: u32,
}

impl TrackSession {
    /// Protect a raw RTP-payload frame and stamp its sequence number.
    pub fn protect_frame(&mut self, kind: MediaFrameKind, data: &[u8]) -> Vec<u8> {
        // RTP header (12 bytes) prefix so the packet is a valid RTP packet that
        // SRTP can protect per RFC 3711, with the payload following it.
        let mut rtp_packet = Vec::with_capacity(12 + data.len());
        let vpx = 0x80u8; // version 2, no padding/extension/CSRC
        let payload_type = match kind {
            MediaFrameKind::Video => 96,
            MediaFrameKind::Screen => 98,
            MediaFrameKind::Voice => 111,
        };
        rtp_packet.push(vpx | (payload_type as u8 & 0x7f));
        rtp_packet.push(self.seq.to_be_bytes()[0]);
        rtp_packet.push(self.seq.to_be_bytes()[1]);
        rtp_packet.extend_from_slice(&self.rtp_timestamp.to_be_bytes());
        rtp_packet.extend_from_slice(&self.ssrc.to_be_bytes());
        rtp_packet.extend_from_slice(data);

        let protected = self.srtp.protect_rtp(&rtp_packet);
        self.seq = self.seq.wrapping_add(1);
        self.rtp_timestamp = self.rtp_timestamp.wrapping_add(160); // ~3.3 ms of 48 kHz audio per frame
        protected
    }

    /// Strip the RTP header, unprotect, and return the bare payload.
    pub fn unprotect_frame(&mut self, protected: &[u8]) -> Option<Vec<u8>> {
        if protected.len() < 12 {
            return None;
        }
        let rtp_packet = self.srtp.unprotect_rtp(protected).ok()?;
        Some(rtp_packet[12..].to_vec())
    }

    /// Advance the RTP timestamp when a frame is skipped (e.g. dropped audio).
    pub fn advance(&mut self, samples: u32) {
        self.rtp_timestamp = self.rtp_timestamp.wrapping_add(samples);
    }
}

/// A per-call pipeline: one track for each media kind, all keyed to the call's
/// SRTP master key.
pub struct MediaPipeline {
    pub session_key: [u8; 32],
    pub tracks: TrackSessions,
}

#[derive(Debug, Default)]
pub struct TrackSessions {
    pub voice: Option<TrackSession>,
    pub video: Option<TrackSession>,
    pub screen: Option<TrackSession>,
}

impl MediaPipeline {
    pub fn new(session_key: [u8; 32]) -> Self {
        Self {
            session_key,
            tracks: TrackSessions::default(),
        }
    }

    /// Create or lazily build the SRTP contexts for all available tracks using
    /// distinct SSRCs so the far end can demultiplex.
    pub fn ensure_tracks(&mut self, base_ssrc: u32) {
        let profile = super::SrtpProfile::Aes256CtrHmacSha1_80;
        if self.tracks.voice.is_none() {
            let srtp = SrtpContext::from_shared_secret(
                &self.session_key,
                0,
                base_ssrc,
                profile,
            );
            self.tracks.voice = Some(TrackSession {
                ssrc: base_ssrc,
                srtp,
                seq: 0,
                rtp_timestamp: 0,
            });
        }
        if self.tracks.video.is_none() {
            let srtp = SrtpContext::from_shared_secret(
                &self.session_key,
                0,
                base_ssrc + 1,
                profile,
            );
            self.tracks.video = Some(TrackSession {
                ssrc: base_ssrc + 1,
                srtp,
                seq: 0,
                rtp_timestamp: 0,
            });
        }
        if self.tracks.screen.is_none() {
            let srtp = SrtpContext::from_shared_secret(
                &self.session_key,
                0,
                base_ssrc + 2,
                profile,
            );
            self.tracks.screen = Some(TrackSession {
                ssrc: base_ssrc + 2,
                srtp,
                seq: 0,
                rtp_timestamp: 0,
            });
        }
    }

    /// Protect a frame for outbound transmission over [`super::send_media_frame`].
    pub fn protect(&mut self, frame: CapturedFrame) -> Option<Vec<u8>> {
        let track = match frame.kind {
            MediaFrameKind::Voice => self.tracks.voice.as_mut(),
            MediaFrameKind::Video => self.tracks.video.as_mut(),
            MediaFrameKind::Screen => self.tracks.screen.as_mut(),
        };
        let track = track?;
        if frame.data.len() > MAX_PAYLOAD_BYTES {
            return None;
        }
        track.rtp_timestamp = frame.rtp_timestamp;
        Some(track.protect_frame(frame.kind, &frame.data))
    }

    /// Unprotect an inbound protected packet into a [`CapturedFrame`] keyed by
    /// the SSRC embedded in the RTP header.
    pub fn unprotect(&mut self, ssrc: u32, protected: &[u8]) -> Option<CapturedFrame> {
        if protected.len() < 12 {
            return None;
        }
        // Peek payload type from the RTP header: [1..2)
        let pt = protected[1] & 0x7f;
        let kind = match pt {
            96 => MediaFrameKind::Video,
            98 => MediaFrameKind::Screen,
            111 | 112 => MediaFrameKind::Voice,
            _ => return None,
        };
        let rtp_ts = u32::from_be_bytes([
            protected[4],
            protected[5],
            protected[6],
            protected[7],
        ]);
        let track = match kind {
            MediaFrameKind::Video => self.tracks.video.as_mut(),
            MediaFrameKind::Screen => self.tracks.screen.as_mut(),
            MediaFrameKind::Voice => self.tracks.voice.as_mut(),
        };
        let payload = track?.unprotect_frame(protected)?;
        Some(CapturedFrame::new(kind, rtp_ts, ssrc, payload))
    }
}

/// Convert a mono PCM sample count at 48 kHz into an RTP timestamp delta.
pub fn pcm_to_rtp_timestamp(samples: usize) -> u32 {
    let bytes_per_ms = (AUDIO_CLOCK_RATE as usize) / 1000;
    let ms = samples / 48;
    (bytes_per_ms as u32) * (ms as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_header_round_trip() {
        let h = FrameHeader::from(1, 0, 12, 90000, 4096);
        let bytes = serde_json::to_vec(&h).unwrap();
        let back: FrameHeader = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(h, back);
    }

    #[test]
    fn pipeline_protect_unprotect_round_trip() {
        let key = [7u8; 32];
        let mut tx = MediaPipeline::new(key);
        tx.ensure_tracks(100);
        let mut rx = MediaPipeline::new(key);
        rx.ensure_tracks(100);

        let payload = vec![0xAA; 1024];
        let protected = tx
            .protect(CapturedFrame::new(
                MediaFrameKind::Video,
                90000,
                101,
                payload.clone(),
            ))
            .expect("protect should succeed");

        let out = rx
            .unprotect(101, &protected)
            .expect("unprotect should succeed");
        assert_eq!(out.data, payload);
    }
}
