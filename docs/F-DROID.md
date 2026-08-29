# Publishing Vchat on F-Droid

Vchat is a 100% free and open-source app (GPL-3.0-only). This document
describes how the project is made ready for distribution through the F-Droid
app store, and exactly what a maintainer must do to publish a release.

## The build model

All builds happen on GitHub Actions (`.github/workflows/build.yml`). The repo is
developed on low-power mobile devices, so **no local toolchain is ever
required**. The CI pipeline is the single authority for:

- frontend type-checking (`tsc`) and bundling (`vite`)
- Rust formatting, clippy and tests
- desktop bundles (Linux, Windows, macOS)
- the Android APK, built as a **single `universal` APK** (all four ABIs:
  `aarch64`, `armv7`, `x86_64`, `i686`)

## Why the Android project is reproducible

1. **Pinned Rust toolchain** — `rust-toolchain.toml` pins
   `channel = "1.91.0"` with `rustfmt`/`clippy`. CI (`dtolnay/rust-toolchain`)
   and the F-Droid recipe install exactly this channel.
2. **Committed `Cargo.lock`** — the `.github/workflows/sync-android.yml`
   workflow generates `src-tauri/Cargo.lock` on GitHub and commits it, so every
   Rust dependency is pinned by checksum.
3. **Deterministic Android project** — `scripts/android-prepare.sh`:
   - bootstraps `src-tauri/gen/android` with `npx tauri android init` if
     missing (idempotent),
   - injects the required runtime permissions into `AndroidManifest.xml`
     (`CAMERA`, `RECORD_AUDIO`, `INTERNET`) with no manual edits,
   - derives the numeric `versionCode` from `src-tauri/Cargo.toml`
     (`MAJOR*1_000_000 + MINOR*1_000 + PATCH`), e.g. `2.0.0 → 2000000`,
   - enables AGP reproducible flags
     (`isPreserveFileTimestamps = false`, `isConservativeR8 = true`).
4. **Committing the generated project** — the sync workflow commits
   `src-tauri/gen/android` to the repository when it changes, so F-Droid and CI
   build from the exact same committed Gradle project.

## Release procedure

1. Bump the version in `src-tauri/Cargo.toml` (and `package.json`) to the same
   value, e.g. `2.0.1`.
2. Push a signed tag `v2.0.1`.
3. CI builds everything and uploads artifacts to a draft GitHub Release:
   - desktop bundles for each OS,
   - `vchat-android-universal.apk` — installable APK (debug-signed, or signed
     with your release keystore when the secrets below are configured),
   - `vchat-android-universal-unsigned.apk` — the identical APK before
     signing, used to verify reproducibility against F-Droid's rebuild.
4. Review, publish the Release.

## Signing (optional, recommended for your own distribution)

You don't need your own signing key for F-Droid (F-Droid signs rebuilt APKs
with its own key). For direct installs from GitHub Releases you can sign with a
release key:

```bash
# generate once, keep it safe
keytool -genkeypair -v -keystore vchat-release.jks \
        -alias vchat -keyalg RSA -keysize 4096 -validity 10000
base64 -w0 vchat-release.jks > vchat-release.jks.b64   # store the .b64 value
```

Then add these GitHub repository secrets:
`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEYSTORE_ALIAS`,
`ANDROID_KEYSTORE_KEY_PASSWORD`. CI signs the APK when the base64 secret is set.

## F-Droid metadata

The proposed metadata recipe lives at `fdroid/org.vchat.messenger.yml`. It is
lint-checked on every push by the `fdroid-check` job. To submit Vchat to the
official F-Droid catalog, open a merge request against
https://gitlab.com/fdroid/fdroiddata with that file; it should be placed at
`metadata/org.vchat.messenger.yml`.

### F-Droid build-server prerequisites

Because Vchat uses Tauri 2, the F-Droid build server needs what a plain Gradle
build alone would not provide:

- **Rust toolchain** `1.91.0` (via `rustup` in the recipe) and the four Android
  cargo targets,
- **Node.js** to build the bundled frontend,
- network access to `static.rust-lang.org`, `crates.io`, and
  `plugins.gradle.org` (the Tauri Gradle plugin is fetched at build time).

These are covered by the `build:` steps of the recipe; if the build server
policy needs the Rust toolchain installed differently, keep the chained
`rustup` steps in `fdroid/org.vchat.messenger.yml` in sync with
`rust-toolchain.toml`.

## Verifying the APK

```bash
# permissions end up exactly as intended
$ANDROID_HOME/build-tools/*/aapt dump permissions app-universal-release.apk
# or
$ANDROID_HOME/build-tools/*/apkanalyzer manifest permissions app-universal-release.apk

# signature is valid / consistent
$ANDROID_HOME/build-tools/*/apksigner verify --verbose app-universal-release.apk
```

## Reproducibility check

Once F-Droid publishes a build, compare it byte-for-byte with the CI
`vchat-android-universal-unsigned.apk` (same version tag = same input commit).