# Build Metadata

## Build Process

Vchat is built using Tauri 2.x with a Rust backend and TypeScript frontend.

### Dependencies

All dependencies are open-source:

**Rust (src-tauri/Cargo.toml):**
- tauri (MIT/Apache-2.0)
- tokio (MIT)
- rusqlite (MIT)
- x25519-dalek (MIT/Apache-2.0)
- ed25519-dalek (MIT/Apache-2.0)
- aes-gcm (MIT/Apache-2.0)
- hkdf (MIT/Apache-2.0)
- sha2 (MIT/Apache-2.0)
- serde/serde_json (MIT/Apache-2.0)
- uuid (MIT/Apache-2.0)
- chrono (MIT/Apache-2.0)
- hex (MIT/Apache-2.0)
- base64 (MIT/Apache-2.0)
- url (MIT/Apache-2.0)
- arti-client (MIT/Apache-2.0)
- qrcode (MIT)
- tracing/tracing-subscriber (MIT)
- anyhow (MIT/Apache-2.0)
- zeroize (MIT/Apache-2.0)
- rand (MIT/Apache-2.0)
- once_cell (MIT/Apache-2.0)
- sha2 (MIT/Apache-2.0)

**Frontend (package.json):**
- @tauri-apps/api (MIT/Apache-2.0)
- @tauri-apps/cli (MIT/Apache-2.0)
- typescript (Apache-2.0)
- vite (MIT)

No proprietary dependencies, no tracking, no analytics.

### Reproducible Builds

The CI/CD pipeline (.github/workflows/build.yml) uses fixed toolchain versions:
- Rust: stable
- Node.js: 20
- Android NDK: 26

### Permissions

The app requests no special permissions on Android. Tor connectivity is handled entirely in-process via the embedded Arti client (pure Rust Tor implementation).

### Data Storage

All data is stored locally in an SQLite database (vchat.db). No data is transmitted to any server.
The app communicates exclusively via Tor onion services.

### Encryption

- End-to-end encryption: AES-256-GCM
- Key exchange: X25519 ECDH
- Key derivation: HKDF-SHA256
- Signing: Ed25519
- Handshake: Noise_XX_25519_ChaChaPoly_BLAKE2s
- Onion services: v3 (ed25519)
