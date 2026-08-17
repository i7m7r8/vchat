# Vchat

Ultra-secure P2P Tor-based messaging, video calling, and screen sharing.

## Architecture

```
┌─────────────────────────────────────────────────┐
│  Tauri App (Android / Desktop)                   │
│                                                  │
│  ┌──────────┐  ┌──────────┐  ┌───────────────┐ │
│  │ Frontend  │  │ WebRTC   │  │ E2E Crypto    │ │
│  │ (TS/CSS) │←→│ (str0m)  │←→│ (snow+AES)    │ │
│  └──────────┘  └──────────┘  └───────────────┘ │
│                     ↕                            │
│  ┌──────────────────────────────────────────┐   │
│  │  Rust Backend                            │   │
│  │  • arti-client (Tor)                     │   │
│  │  • Onion service (signaling)             │   │
│  │  • Custom TCP transport via Tor          │   │
│  └──────────────────────────────────────────┘   │
└─────────────────────────────────────────────────┘
```

## Features

- **P2P Communication**: No central servers, direct peer-to-peer
- **Tor Onion Services**: Each peer runs a Tor hidden service
- **E2E Encryption**: Noise Protocol (XX pattern) + AES-256-GCM
- **Video Calling**: WebRTC-based video/audio calls
- **Screen Sharing**: Real-time screen sharing during calls
- **Cross-Platform**: Android first, then Desktop (Windows/Mac/Linux)
- **QR Code Contact Exchange**: Share QR codes to add contacts

## Security Model

1. **Key Exchange**: Noise XX pattern with X25519
2. **Message Encryption**: AES-256-GCM with derived shared keys
3. **Transport**: All traffic routed through Tor onion services
4. **No Metadata**: No central servers to log metadata
5. **Forward Secrecy**: Ephemeral keys for each session

## Tech Stack

| Component | Technology |
|-----------|------------|
| App Framework | Tauri 2.x |
| Backend | Rust |
| Frontend | TypeScript + CSS |
| Tor | Arti (pure Rust) |
| Encryption | Snow (Noise Protocol) + AES-GCM |
| WebRTC | str0m (sans-I/O) |
| Database | SQLite (via rusqlite) |

## Prerequisites

- Rust 1.70+
- Node.js 18+
- Android SDK + NDK (for Android builds)
- Tauri CLI

## Development

```bash
# Install dependencies
npm install

# Run in development mode
cargo tauri dev

# Build for Android
cargo tauri android dev

# Build APK
cargo tauri android build
```

## Project Structure

```
vchat/
├── src-tauri/           # Rust backend
│   ├── src/
│   │   ├── main.rs      # Entry point
│   │   ├── lib.rs       # Tauri setup
│   │   ├── commands.rs  # Tauri commands
│   │   ├── tor/         # Tor onion service
│   │   ├── crypto/      # E2E encryption
│   │   ├── webrtc/      # Video/audio
│   │   └── messaging/   # Message handling
│   └── Cargo.toml
├── src/                 # Frontend
│   ├── main.ts          # App entry
│   ├── lib/api.ts       # Tauri API wrapper
│   └── lib/store.ts     # State management
├── index.html           # UI
├── styles.css           # Styling
└── package.json
```

## Security Considerations

- All encryption keys are generated locally
- No data is ever sent to central servers
- Tor onion services provide network-level anonymity
- Messages are encrypted end-to-end before transmission
- SQLite database is stored locally on device

## License

MIT
