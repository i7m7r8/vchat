# vchat P2P Architecture (Jami-style)

## Overview

This document describes the Jami-inspired P2P architecture for vchat, replacing the Tor-based design with a pure P2P approach using:
- **OpenDHT** for peer discovery and signaling
- **ICE/STUN/TURN** for NAT traversal
- **TLS/DTLS** for encrypted control channel
- **SRTP** for secure media transport
- **Git-based swarm** for message/file synchronization
- **DRT** for efficient P2P routing

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           vchat P2P Architecture                             │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐                   │
│  │   Peer A    │     │  OpenDHT    │     │   Peer B    │                   │
│  │             │◄────┤  (DHT Net)  │────►│             │                   │
│  │  Identity   │     │  Bootstrap  │     │  Identity   │                   │
│  │  (Ed25519)  │     │  Nodes      │     │  (Ed25519)  │                   │
│  └──────┬──────┘     └─────────────┘     └──────┬──────┘                   │
│         │                                        │                          │
│         │  1. Publish identity/presence          │                          │
│         │  2. Find peer via DHT                  │                          │
│         │  3. Exchange ICE candidates            │                          │
│         ▼                                        ▼                          │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                    ICE NAT Traversal                                 │   │
│  │  • STUN for reflexive addresses                                      │   │
│  │  • TURN for relayed candidates (fallback)                            │   │
│  │  • UDP/TCP candidates                                                │   │
│  │  • ICE connectivity checks                                           │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│         │                                        │                          │
│         ▼                                        ▼                          │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │              TLS/DTLS Control Channel                                │   │
│  │  • TCP → TLS 1.3                                                     │   │
│  │  • UDP → DTLS 1.3                                                    │   │
│  │  • Mutual authentication (Ed25519 certs)                             │   │
│  │  • Multiplexed: SIP signaling + text messages + file metadata        │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│         │                                        │                          │
│         ▼                                        ▼                          │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │              SRTP Media Transport                                    │   │
│  │  • Audio: Opus                                                       │   │
│  │  • Video: VP9/H.264                                                  │   │
│  │  • Screenshare: VP9                                                  │   │
│  │  • Keys derived from DTLS-SRTP                                       │   │
│  │  • Multi-stream support (audio_0, video_0, video_1, etc.)            │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │              Git-based Swarm (Background)                            │   │
│  │  • Message history as Git commits                                    │   │
│  │  • File metadata in Git                                              │   │
│  │  • Group sync via Git pull/push                                      │   │
│  │  • DRT for efficient routing                                         │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Component Details

### 1. Identity System
- **Ed25519 key pair** per device
- **Certificate chain**: CA → Account → Device (like Jami)
- **Identity string**: `base64(ed25519_pubkey)` = 43 chars
- **Device certificate**: Signed by account key, includes device name

### 2. OpenDHT Integration
- **Bootstrap nodes**: Hardcoded list of known DHT nodes
- **Presence publishing**: `put(key, value)` every 30s
  - Key: `hash(identity + ":presence")`
  - Value: `{ip:port, ICE candidates, timestamp, version}`
- **Peer lookup**: `get(hash(peer_id + ":presence"))`
- **Encrypted messaging**: For ICE candidate exchange

### 3. ICE NAT Traversal
```
Candidate Types (priority order):
1. Host (local interface IP:port) - priority 126
2. Server Reflexive (STUN) - priority 100
3. Relayed (TURN) - priority 1

ICE Process:
1. Gather all candidates
2. Exchange via DHT (encrypted)
3. Connectivity checks (STUN binding requests)
4. Nominate working pair
5. Media flows on nominated path
```

### 4. TLS/DTLS Control Channel
- **TCP path**: TLS 1.3 with mutual auth
- **UDP path**: DTLS 1.3 with mutual auth
- **Certificate verification**: Ed25519 chain validation
- **Multiplexing**: Single socket carries:
  - SIP signaling (INVITE, BYE, media negotiation)
  - Text messages
  - File transfer metadata
  - Media control (rotation, keyframe requests)

### 5. SRTP Media Transport
- **Key derivation**: DTLS-SRTP (RFC 5764)
- **Audio**: Opus @ 48kHz, 20ms frames
- **Video**: VP9 preferred, H.264 fallback
- **Screenshare**: VP9 @ 15-30fps
- **Multi-stream**: Up to 32 streams (audio_0, video_0, video_1...)

### 6. Git-based Swarm
- **Per-conversation Git repo**
- **Structure**:
  ```
  /
  ├─ admins/           # Admin public keys
  ├─ members/          # Member public keys
  ├─ devices/          # Device certificates
  ├─ invited/          # Pending invites
  ├─ banned/           # Banned certificates
  ├─ votes/            # Ban/unban votes
  ├─ CRLs/             # Certificate revocation lists
  ├─ messages/         # Commit per message
  ├─ files/            # File metadata
  └─ profile.vcf       # Conversation metadata
  ```
- **Sync**: Git pull/push over TLS channel
- **DRT**: Routes sync requests efficiently

### 7. Distributed Routing Table (DRT)
- **Kademlia-style** binary tree with k-buckets
- **Distance**: XOR metric on node IDs
- **Bootstrap**: Known devices from conversation
- **Maintenance**: Periodic FIND requests
- **Mobile nodes**: Flag for battery optimization

### 8. File Transfer
- **Metadata in Git**: `{tid, displayName, totalSize, sha3sum, type}`
- **Data**: Direct P2P over SRTP (or separate TCP stream)
- **Resume**: Range requests supported
- **Multi-source**: Download from multiple peers

### 9. Screenshare
- **Media negotiation**: `requestMediaChange` API
- **New stream**: Negotiate 2 new UDP sockets via ICE
- **Encoding**: VP9, configurable resolution/bitrate

## Security Model

1. **Forward Secrecy**: Ephemeral keys per session (DTLS-SRTP)
2. **Authentication**: Ed25519 mutual auth on every connection
3. **Message Integrity**: Git commit signatures + SRTP auth tags
4. **Perfect Forward Secrecy**: New DTLS handshake per call
5. **No Metadata Collection**: No central servers, no tracking

## Protocol Stack

```
Application Layer
├── Messaging API
├── Call API (audio/video/screenshare)
├── File Transfer API
├── Group/Swarm API
│
├── SIP Signaling (over TLS/DTLS)
├── Media Control (XML over SIP)
├── Git Swarm Protocol
│
Transport Layer
├── TLS 1.3 (TCP control)
├── DTLS 1.3 (UDP control)
├── SRTP (media)
├── ICE (NAT traversal)
│
Network Layer
├── UDP (primary for media)
├── TCP (fallback for control)
├── OpenDHT (peer discovery)
└── IPv4/IPv6
```

## Build Requirements

### Rust Dependencies (Cargo.toml)
```toml
# DHT
opendht = { version = "2.0", features = ["tokio"] }

# ICE/STUN/TURN
stun-rs = "0.2"
turn = "0.1"
ice = "0.3"

# TLS/DTLS
rustls = { version = "0.23", features = ["std", "dangerous_configuration", "tls12", "tls13"] }
webpki = "0.23"
srtp = "0.2"

# Git
git2 = { version = "0.18", features = ["ssh", "https"] }

# Media
opus = "0.5"
vpx = "0.5"  # VP9 encoding

# Existing
x25519-dalek = "2"
ed25519-dalek = "2"
aes-gcm = "0.10"
hkdf = "0.12"
snow = "0.10"  # Noise protocol (optional, for additional handshake)
```

## Implementation Phases

### Phase 1: Core P2P Infrastructure
- [ ] OpenDHT client integration
- [ ] Identity management (Ed25519 + certs)
- [ ] ICE candidate gathering
- [ ] STUN/TURN client

### Phase 2: Secure Transport
- [ ] TLS/DTLS control channel
- [ ] SRTP media transport
- [ ] DTLS-SRTP key derivation
- [ ] Media pipeline (Opus + VP9)

### Phase 3: Signaling & Calls
- [ ] SIP signaling over TLS/DTLS
- [ ] ICE connectivity checks
- [ ] Audio call implementation
- [ ] Video call implementation

### Phase 4: Advanced Features
- [ ] Screenshare
- [ ] Multi-stream
- [ ] File transfer
- [ ] Group calls (conference)

### Phase 5: Swarm & Sync
- [ ] Git-based conversation repo
- [ ] DRT implementation
- [ ] Message sync
- [ ] File transfer

### Phase 6: Polish & F-Droid
- [ ] Reproducible builds
- [ ] F-Droid metadata
- [ ] Performance optimization
- [ ] Battery optimization

## F-Droid Compliance

- All dependencies: MIT/Apache-2.0/GPL-3.0 compatible
- No proprietary blobs
- Reproducible builds via pinned toolchain
- Source-only distribution