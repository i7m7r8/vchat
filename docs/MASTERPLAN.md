# vchat Masterplan — Truly Serverless P2P Messenger

**Status:** Masterplan (research + phased roadmap)
**Owner:** vchat repo
**Build authority:** GitHub Actions CI (`.github/workflows/build.yml`) — nothing builds locally
**License:** GPL-3.0-only

---

## 0. Why this document exists

vchat is currently a **Tor-based** serverless messenger: each peer runs an embedded
Arti (pure-Rust Tor) onion service, and messages travel peer-to-peer through the
Tor network to reach the peer's onion address. This is genuinely **0-server** and
**100% pure Rust** on the transport path, but it does not yet implement Jami's
**direct p2p** model (DHT discovery + ICE hole-punching + SRTP media) nor Signal's
**X3DH + Double Ratchet** encryption.

This masterplan records the research, the target architecture, and the phased,
CI-driven implementation plan to make vchat a **Jami + Signal**-grade messenger.

---

## 1. Research summary

### 1.1 How Jami is truly serverless

Jami (Savoir-faire Linux) is a fully decentralized, peer-to-peer messenger with
**no servers**. Its key mechanisms:

1. **Identity (account) system**
   - Each user has a GNU Ring/Jami account backed by an Ed25519 master key.
   - A certificate chain `CA → Account → Device` binds multiple devices to one
     identity. Each device has its own keypair and a signed device certificate.

2. **Peer discovery via OpenDHT (Kademlia DHT)**
   - Jami runs/joins an **OpenDHT** network — a Kademlia-style distributed hash
     table. There is no central index; the bootstrap nodes are just well-known
     entry points into the decentralized mesh.
   - Each peer **publishes its presence** (identity hash + reachable endpoints)
     and its **ice candidates** to the DHT under a key derived from its identity.
   - To call/message a contact, a peer **looks up** the contact's DHT key to learn
     how to reach it (endpoints + latest ICE candidates).
   - Because the DHT is distributed and self-healing, it does not constitute a
     server; anyone can run a bootstrap node and the network keeps working.

3. **NAT traversal — ICE with STUN/TURN**
   - Direct connectivity is established with **ICE** (RFC 8445): each peer gathers
     **host** candidates (local IP), **server-reflexive** candidates via **STUN**,
     and, only as a fallback, **relayed** candidates via **TURN**.
   - Candidates are exchanged out-of-band (over the DHT / DHT-messaging in newer
     designs). ICE connectivity checks then **hole-punch** a direct UDP path.
   - vchat already has a **hand-rolled, dependency-free STUN/ICE/TURN client** in
     `src-tauri/src/webrtc/ice.rs` — the media path directly parallels Jami.

4. **Swarm (group + sync) model — Git-based repositories**
   - Modern Jami replaced server-side groups with a **"swarm"**: each conversation
     is a **Git repository** whose history IS the conversation.
   - Structure: `admins/`, `members/`, `devices/`, `invited/`, `banned/`, `CRLs/`,
     `messages/` (one commit per message), `files/` (metadata), `profile.vcf`.
   - Peers **sync the Git repo peer-to-peer**; messages are commits verified by
     device certificates. This gives offline-first delivery and multi-device sync
     without any server.
   - vchat's `docs/P2P_ARCHITECTURE.md` already describes exactly this swarm model
     and a DRt (Distributed Routing Table) on top.

5. **Media — SRTP/RTCP over UDP**
   - Audio (Opus @48kHz), video (VP8/VP9/H.264), and screen share are carried over
     **SRTP** on ICE-established UDP paths. DTLS-SRTP is used for key derivation.
   - Conference calls multiplex multiple SRTP streams.

**Key takeaway:** "No server" is achieved by three pillars — (a) **DHT discovery**,
(b) **ICE hole-punching** for direct transport, and (c) **Git-based swarm** sync.
Nothing requires a central authority; bootstrap nodes and STUN/TURN relays are
optional accelerants, not servers (no messages or metadata route through them).

### 1.2 Signal's encryption model

Signal uses two layered protocols:

1. **X3DH (Extended Triple Diffie-Hellman)** — for the initial key agreement.
   - Parties: Alice (initiator), Bob (recipient, possibly offline), and **a server**
     that stores Bob's published prekeys. **For vchat, the role of this "server"
     is played by the DHT** — Bob publishes his prekeys to the DHT; Alice fetches
     them from the DHT; no central server is involved.
   - Keys: Identity key (`IK`), signed prekey (`SPK`, rotated periodically),
     one-time prekeys (`OPK`, single use), ephemeral key (`EK`).
   - Shared secret: `SK = KDF(DH1 || DH2 || DH3 || DH4)` where
     `DH1=DH(IK_A,SPK_B)`, `DH2=DH(EK_A,IK_B)`, `DH3=DH(EK_A,SPK_B)`,
     `DH4=DH(EK_A,OPK_B)`.
   - Provides mutual authentication + **forward secrecy** + **deniability**.
   - vchat currently does plain X25519 ECDH + HKDF + AES-256-GCM per message; this
     gives no forward secrecy across the session and no one-time prekey use.

2. **Double Ratchet** — for ongoing per-message encryption.
   - Combines a **DH ratchet** (each party rotates a new ephemeral key per message
     round) with a **symmetric-key (KDF) ratchet** to derive a fresh key per message.
   - Provides **future secrecy** (compromise of current keys doesn't expose past
     messages) and **post-compromise security** (recovery after key compromise).
   - Uses HKDF chains and associated data (header) for authentication.
   - Crates: `x3dh` (not stable) and `double-ratchet` / `ratchet` exist; a safe,
     audited choice is `double-ratchet` crate or a hand-rolled HKDF ratchet (vchat
     already depends on `hkdf`).

### 1.3 Current vchat state (from repo audit)

Working & pure-Rust (serverless via Tor):
- `identity/`, `crypto/` (Ed25519, X25519, AES-256-GCM, HKDF, Noise XX, PQC hybrid)
- `tor/` (Arti onion services) + `messaging/` wire protocol + `commands.rs`
- `webrtc/` + `webrtc/ice.rs` (hand-rolled STUN/ICE/TURN + UDP media)
- Frontend `src/` (Tauri 2 + TypeScript) + GitHub Actions CI

Aspirational / not yet compiling (the Jami DHT/SRTP/swarm layer):
- `dht/`, `ice/`, `transport/`, `signaling/`, `swarm/`, `drt/` — declared in
  `lib.rs` but refer to crate APIs that do not exist (opendht "2.0", `stun`,
  `srtp`, `git2` at wrong versions), and the repo does **not** currently compile.

---

## 2. Target architecture (Jami + Signal for vchat)

```
 Peer A (device)                          Peer B (device)
 ┌──────────────────────┐                 ┌──────────────────────┐
 │  Identity/Keys       │                 │  Identity/Keys       │
 │  Ed25519 + X25519    │                 │  Ed25519 + X25519    │
 │  (vchat identity)    │                 │                      │
 ├──────────────────────┤                 ├──────────────────────┤
 │  X3DH prekey bundle  │                 │  X3DH prekey bundle  │
 │  + Double Ratchet    │  ◄── DHT ────►  │  + Double Ratchet    │
 │  (encryption core)   │                 │  (encryption core)   │
 ├──────────────────────┤                 ├──────────────────────┤
 │  OpenDHT client      │  ◄── DHT ────►  │  OpenDHT client      │
 │  presence + prekeys  │    (Kademlia)   │  presence + prekeys  │
 ├──────────────────────┤                 ├──────────────────────┤
 │  ICE/STUN/TURN       │  ◄── UDP ────►  │  ICE/STUN/TURN       │
 │  (webrtc/ice.rs)     │   (hole-punch)  │  (webrtc/ice.rs)     │
 ├──────────────────────┤                 ├──────────────────────┤
 │  SRTP media          │  ◄── SRTP ────► │  SRTP media          │
 │  Opus/VP9/VP9 screen │    over UDP     │  Opus/VP9/VP9 screen │
 ├──────────────────────┤                 ├──────────────────────┤
 │  Git swarm repos     │  ◄── Git sync ─►│  Git swarm repos     │
 │  (messages/groups)   │    (peer2peer)  │  (messages/groups)   │
 └──────────────────────┘                 └──────────────────────┘
        Bootstrap nodes / STUN / TURN = only OPTIONAL accelerants (not servers)
```

**Migration strategy (honest):** The repo's working core is the Tor/wire-protocol
messenger. The target is a **dual-transport** design for a smooth, non-breaking
migration:

- **Transport**
  - `TOR` (default, already works): messages over Arti onion services.
  - `DIRECT` (Jami-style, target): messages over ICE-established UDP/TCP paths,
    discovered via OpenDHT.
  - A transport trait abstracts both; call/media always prefer `DIRECT`, falling
    back to `TOR`.
- **Encryption** upgrades in place: add X3DH + Double Ratchet (forward secrecy),
  keeping AES-256-GCM but deriving per-message keys via a ratchet. Wire version
  bump for protocol negotiation.
- **Discovery** moves from "Tor onion lookup" to "OpenDHT presence lookup" while
  retaining Tor fallback.

---

## 3. Phased implementation plan (CI-driven)

Every phase ships a **compiling, CI-green increment** to `main`. All compilation,
linting, tests and artifact builds run **only in GitHub Actions**.

### Phase 0 — Unbreak the build (do this first; in progress)
Goal: repo compiles clean under `cargo check/clippy/test --all-features` in CI.
- [x] Fix `Cargo.toml`: remove duplicate dep blocks, fix self-referential features,
      use real crate names/versions (`pqc_kyber`, `pqc_dilithium`, `pqc_sphincsplus`,
      `rustls` no `tls13` feature, etc.).
- [x] Fix `lib.rs`: no duplicate `crypto` module; declare `pub mod tor`; gate the
      not-yet-real Jami modules (`dht`, `ice`, `transport`, `signaling`, `swarm`)
      behind a `jami-p2p` opt-in feature so the default build is green.
- [ ] Restore/resolve arti deps and make the core Tor messenger compile.
- [ ] Fix `crypto/pqc.rs` (remove `ring::kem` fallback; use the pure-Rust pqc crates
      or x25519/ed25519 fallbacks).
- [ ] Add CI status badge; require `lint-and-test` green before merge.
- Outcome: default build green on GitHub; existing features work.

### Phase 1 — Signal-grade encryption (X3DH + Double Ratchet)
Goal: forward secrecy + per-message keys, replacing static DH per message.
- [ ] X3DH: prekey bundle type, prekey rotation, `SK` derivation (X25519+HKDF).
- [ ] Publish prekeys to the DHT (or Tor store) and fetch contact prekey bundles.
- [ ] Double Ratchet: DH ratchet + symmetric KDF ratchet; header encryption
      (`crypto/ratchet.rs`). Use `hkdf` (already a dep); hand-roll to minimize deps.
- [ ] Session store (SQLite): ratchet state per conversation, persist + recover.
- [ ] Wire protocol bump (`WIRE_VERSION = 2`) with negotiation.
- Tests: property tests for ratchet forward/backward secrecy; two-session interop.

### Phase 2 — OpenDHT discovery (serverless presence + prekeys)
Goal: replace central lookup with a Kademlia DHT (Jami-style).
- [ ] Real OpenDHT integration (C++ lib via its Rust binding) — built **only in CI**
      with native OpenDHT; enabled behind `jami-p2p` feature with a documented
      build step. Pure-Rust fallback: a minimal internal Kademlia DHT crate behind
      the same trait (keeps "0 server" without mandatory C++).
- [ ] Publish presence {identity, endpoints, ICE candidates, version} every 30s.
- [ ] Lookup flow: resolve contact → prekey bundle + ICE candidates.
- [ ] Bootstrap node list config; optional self-hosted bootstrap.
- Tests: two in-process DHT nodes find each other; persistence across restarts.

### Phase 3 — Direct ICE + SRTP media (calls, video, screen share)
Goal: low-latency direct media, matching Jami.
- [ ] Wire the existing `webrtc/ice.rs` (host/srflx/relay gather, connectivity
      checks, hole-punching, UDP framing) into call signalling over DHT/Tor.
- [ ] SRTP layer: DTLS-SRTP key derivation; SRTP protect/unprotect for RTP/RTCP.
      Use `srtp` crate + `webrtc` crates or hand-roll lightweight framing over the
      protected UDP path (vchat's `webrtc::ice` already does encrypted-relevant
      framing — add actual SRTP protection).
- [ ] Opus audio capture/encode/play; VP9 video encode/decode and screen capture —
      gate behind `jami-p2p` and build native media codecs in CI only (or use
      `tauri`/OS media APIs on desktop).
- [ ] Call state machine (ringing/connected/mute/screenshare) integrated with
      `webrtc/mod.rs` (already largely present).
- Tests: loopback media round-trip; candidate prioritization; fallback to Tor.

### Phase 4 — Swarm (Git) group + sync
Goal: decentralized groups and multi-device sync via Git repos (no server).
- [ ] Per-conversation Git repo (structure per `P2P_ARCHITECTURE.md`).
- [ ] Message-as-commit with device-cert verification; conflict resolution.
- [ ] Peer sync over transport (pull/push via DRT/DRt routing).
- [ ] DRT (Distributed Routing Table) for efficient routing of sync/message lookups.
- Tests: two nodes converge the same conversation repo; replay/fork detection.

### Phase 5 — Conference calls (multi-party)
Goal: audio/video conference, like Jami.
- [ ] Multiplexed SRTP streams; SFU-less mesh with selective forwarding
      (each participant N-1 streams), or Jami-style mixer for small groups.
- [ ] Screen share as an additional negotiated stream (`requestMediaChange`).
- Tests: 3–4 in-process peers conference; stream add/remove.

### Phase 6 — Signal-grade features + UI rewrite
- [ ] Disappearing messages TTL (already scaffolded), read receipts, typing
      indicators (present), quote/reply, reactions (present), voice notes (present).
- [ ] Contact verification via QR (present) + safety-number/fingerprint comparison.
- [ ] **UI rewrite** — modern **Jami + Signal** blend: SwiftUI/Signal-style message
      bubbles, smooth spring animations, gesture-driven interactions (swipe to
      reply/call, pull-to-refresh sync), adaptive light/dark theme, large thumbnails,
      blur/vibrancy accents. Implemented in TypeScript+CSS webview; ship in phases
      with visual snapshots in CI (optional Playwright screenshots).

### Phase 7 — Hardening, reproducibility & release
- [ ] Security audit pass: constant-time code, key zeroization (already using
      `zeroize`), transport encryption, ratchet correctness.
- [ ] F-Droid reproducible build (recipe present), deterministic APK, release
      automation (tags → draft release with signed artifacts).
- [ ] Performance/battery: background connection, network wake, caching.
- [ ] Load-testing multidevice sync and large group histories.

---

## 4. Dependency & Israeli-pure-Rust decision

- **Pure-Rust preferred and mandatory for the default build:** Tor (Arti), crypto
  (dalek/RustCrypto), ICE (hand-rolled), SQLite (`rusqlite` bundled).
- **Native-only behind `jami-p2p` feature (built in CI only):** OpenDHT (C++),
  SRTP (`srtp` → libsrtp2), media codecs (Opus/VP9), Git (`git2` → libgit2).
  These are heavyweight and cannot/can-not-compile on the low-end dev device; the
  CI builds them into platform bundles.
- Minimal new dependencies; prefer hand-rolled, auditable implementations for the
  protocol core (ratchet, DHT fallback) to keep the surface small and dependency-
  conflict-free (the ring 0.13/0.16 conflict between OpenDHT and rustls is avoided
  by keeping OpenDHT native and never mixing its ring usage with the TLS stack).

---

## 5. CI build pipeline (authoritative)

`.github/workflows/build.yml` should enforce, on every push/PR to `main`:
1. `lint-and-test` — `cargo fmt`, `clippy --all-targets --all-features -D warnings`,
   `cargo test --all`, frontend `tsc --noEmit` + `vite build`. **Must be green.**
2. `build-{linux,windows,macos,android}` — `tauri build` + Android universal APK
   via `scripts/android-prepare.sh`. Upload artifacts + sign release on tags.
3. `sync-android.yml` — regenerates `Cargo.lock` + `gen/android` so the committed
   Gradle project stays in sync (required for F-Droid reproducibility).
4. Optional per-phase feature builds: `cargo check --features jami-p2p` in a job
   that installs the native OpenDHT/libsrtp2/libgit2 deps, so the heavy layer is
   continuously tested without blocking the default build.

---

## 6. Risks & honest limits

- **"100% production-grade Jami in one session" is not achievable** by any single
  effort — Jami itself is a multi-year, multi-engineer project. This masterplan
  sequences the work into CI-green increments that each deliver a real, working,
  testable slice. Each phase is independently shippable.
- Native (DHT/SRTP/codec) phases require CI machines with the native toolchains;
  this is exactly why the build authority is GitHub Actions, not the dev device.
- The default, fully-pure-Rust path (Tor + hand-rolled ICE + new ratchet) remains
  usable now and is only enhanced by the native phases — so vchat is never blocked
  on heavy native builds.

---

## 7. Immediate next step

Continue **Phase 0**: land the Cargo/lib/gating fixes I've already made, push to
`main`, and confirm the `lint-and-test` job is green in GitHub Actions. Then start
**Phase 1** (X3DH + Double Ratchet) as the first protocol-quality leap.
