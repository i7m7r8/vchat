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

Rust toolchain is pinned in `rust-toolchain.toml` (channel `1.91.0`, with
`rustfmt`/`clippy`). CI and the F-Droid recipe install exactly this channel.

`src-tauri/Cargo.lock` is committed; it is generated once by
`.github/workflows/sync-android.yml` (which also commits the generated
`src-tauri/gen/android` Gradle project when it changes).

The Android build is deterministic:

- fixed Rust toolchain (`rust-toolchain.toml`)
- committed `Cargo.lock` (pinned dependency graph)
- fixed Node.js (22) and `npm` lockfile on CI
- fixed Android SDK/NDK: `NDK 27.0.12077973`, Java 17
- `scripts/android-prepare.sh` injects runtime permissions and derives the
  numeric `versionCode` from `src-tauri/Cargo.toml`
  (`MAJOR*1_000_000 + MINOR*1_000 + PATCH`)
- AGP reproducible flags: `isPreserveFileTimestamps = false`,
  `isConservativeR8 = true`
- single `universal` APK (aarch64 + armv7 + x86_64 + i686) so F-Droid can
  publish one artifact

### Permissions

The app requests only what it needs for calls/networking on Android: `CAMERA`
(mic/video calls), `RECORD_AUDIO`, and `INTERNET` (embedded Tor). Tor
connectivity is handled entirely in-process via the embedded Arti client (pure
Rust Tor implementation), so no extra privileges are required.

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
