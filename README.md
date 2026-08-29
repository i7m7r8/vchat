# Vchat

P2P encrypted messenger with embedded Tor. No servers, no tracking.

## Architecture

```
┌──────────────────────────────────────────────┐
│  Tauri 2.x App                               │
│                                               │
│  Frontend (TypeScript + CSS)                  │
│       ↕ Tauri IPC                            │
│  Backend (Rust)                               │
│  • E2E Encryption (Noise XX + AES-256-GCM)   │
│  • Embedded Tor (Arti - pure Rust)            │
│  • SQLite Database                            │
│  • P2P Messaging via onion services           │
└──────────────────────────────────────────────┘
```

## Features

- **P2P Messaging**: Direct peer-to-peer encrypted messaging via Tor
- **Tor Onion Services**: Each peer runs a Tor hidden service via embedded Arti
- **E2E Encryption**: Noise Protocol (XX pattern) + AES-256-GCM
- **Group Messaging**: Create groups, add members, group chat
- **Reactions**: Emoji reactions on messages
- **Disappearing Messages**: Configurable TTL
- **Read Receipts & Typing Indicators**
- **QR Code Contact Exchange**
- **Cross-Platform**: Android, Linux, Windows, macOS

## Security Model

1. **Key Exchange**: Noise XX pattern with X25519
2. **Message Encryption**: AES-256-GCM with HKDF-derived keys
3. **Transport**: Tor onion services (v3)
4. **No Metadata Collection**: No central servers
5. **Forward Secrecy**: Ephemeral keys per session

## Tech Stack

| Component | Technology |
|-----------|------------|
| App Framework | Tauri 2.x |
| Backend | Rust |
| Frontend | TypeScript + CSS |
| Tor | Arti (embedded, pure Rust - no external daemon) |
| Encryption | snow (Noise) + AES-256-GCM |
| Database | SQLite (rusqlite) |

## Prerequisites

- Rust 1.75+
- Node.js 18+

## Building

All builds happen via GitHub Actions CI (`.github/workflows/build.yml`), which
is also the single-authority build pipeline for the published Android APK.
Development happens on low-power devices, so **no local toolchain is required**;
the CI runs type-checking, tests, lints and every platform bundle.

The Android APK is built as a single `universal` APK (all ABIs) with a
deterministic manifest and versioning (`scripts/android-prepare.sh`), and the
generated Gradle project plus `Cargo.lock` are committed to the repo by
`.github/workflows/sync-android.yml`.

For local development (desktop):
```bash
npm install
npm run tauri dev
```

See [`docs/F-DROID.md`](docs/F-DROID.md) for the F-Droid publishing procedure,
reproducible-build rationale and the submit recipe
([`fdroid/org.vchat.messenger.yml`](fdroid/org.vchat.messenger.yml)).

## Roadmap

See [`docs/MASTERPLAN.md`](docs/MASTERPLAN.md) for the detailed research-backed
roadmap to a Jami-style serverless P2P + Signal-grade encryption messenger:
X3DH + Double Ratchet, OpenDHT discovery, direct ICE/SRTP media, Git-based swarm
groups, conference calls, and a modern Jami + Signal UI. All phases ship as
CI-green increments built by GitHub Actions.

## License

GPL-3.0-only
