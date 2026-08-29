#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# android-prepare.sh
#
# Prepares the committed/generated Android project for a deterministic build.
# It is the single source of truth used by:
#   * .github/workflows/build.yml      (CI APK build)
#   * .github/workflows/sync-android.yml (commits the generated project)
#   * the F-Droid build recipe         (see fdroid/org.vchat.messenger.yml)
#
# When the generated Android project is not present it bootstraps it with
# `npx tauri android init`. This keeps every build environment in lockstep so
# that a given source commit produces a byte-identical APK.
# -----------------------------------------------------------------------------
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

M="src-tauri/gen/android/app/src/main/AndroidManifest.xml"
GRADLE="src-tauri/gen/android/app/build.gradle.kts"

# 1. Bootstrap the Android project when missing (F-Droid build server or first
#    local checkout). Idempotent: refuses to overwrite an existing project.
if [ ! -d src-tauri/gen/android ]; then
  echo "[android-prepare] bootstrapping Android project with tauri android init"
  npx tauri android init
fi

# 2. Patch the manifest with the runtime permissions the app needs
#    (microphone + camera for calls, network for Tor). Idempotent.
PYTHON_BIN="${PYTHON:-python3}"
"$PYTHON_BIN" - "$M" <<'EOF'
import re, sys

manifest = sys.argv[1]
with open(manifest, encoding="utf-8") as fh:
    text = fh.read()

perms = [
    "android.permission.CAMERA",
    "android.permission.RECORD_AUDIO",
    "android.permission.INTERNET",
]

missing = [p for p in perms if f'name="{p}"' not in text]
if missing:
    block = "\n".join(f'    <uses-permission android:name="{p}"/>' for p in missing)
    match = re.search(r"<manifest[^>]*>\n", text)
    if not match:
        sys.exit("android-prepare: could not locate <manifest> element")
    insert_at = match.end()
    text = text[:insert_at] + block + "\n" + text[insert_at:]
    with open(manifest, "w", encoding="utf-8") as fh:
        fh.write(text)
    print(f"[android-prepare] added permissions: {', '.join(missing)}")
else:
    print("[android-prepare] manifest permissions already present")
EOF

# 3. Derive a deterministic numeric versionCode from the Cargo.toml version
#    (MAJOR * 1_000_000 + MINOR * 1_000 + PATCH) and pin versionName.
"$PYTHON_BIN" - "$GRADLE" <<'EOF'
import re, sys

gradle = sys.argv[1]
with open(gradle, encoding="utf-8") as fh:
    text = fh.read()

with open("src-tauri/Cargo.toml", encoding="utf-8") as fh:
    cargo = fh.read()

m = re.search(r'^version\s*=\s*"([^"]+)"\s*$', cargo, re.MULTILINE)
if not m:
    sys.exit("android-prepare: no version found in src-tauri/Cargo.toml")
major, minor, patch = (int(p) for p in m.group(1).split(".")[:3])
version_code = major * 1_000_000 + minor * 1_000 + patch
version_name = m.group(1)

pattern_code = re.compile(r"(?m)^(\s*)versionCode\s*=\s*\d+\s*$")
pattern_name = re.compile(r'(?m)^(\s*)versionName\s*=\s*"[^"]*"\s*$')
if pattern_code.search(text) and pattern_name.search(text):
    text = pattern_code.sub(lambda _: f"        versionCode = {version_code}", text)
    text = pattern_name.sub(lambda _: f'        versionName = "{version_name}"', text)
else:
    # Fallback: inject into defaultConfig when the template uses different names.
    dc_match = re.search(r"defaultConfig\s*\{", text)
    if not dc_match:
        sys.exit("android-prepare: could not locate defaultConfig block")
    base = text[:dc_match.end()]
    rest = text[dc_match.end():]
    if not pattern_code.search(text):
        base += f'\n        versionCode = {version_code}'
    if not pattern_name.search(text):
        base += f'\n        versionName = "{version_name}"'
    text = base + rest

# Reproducible APK settings: deterministic zip entries + deterministic R8.
if "isPreserveFileTimestamps" not in text:
    android_match = re.search(r"android\s*\{", text)
    if android_match:
        indent = len(re.match(r"\s*", text[android_match.end():], re.MULTILINE).group(0)) + 4
        pad = "\n" + " " * indent
        flags = "isPreserveFileTimestamps = false"
        flags += pad + "isConservativeR8 = true"
        text = text[:android_match.end()] + pad + flags + text[android_match.end():]

with open(gradle, "w", encoding="utf-8") as fh:
    fh.write(text)
print(f"[android-prepare] pinned versionName={version_name} versionCode={version_code}")
EOF

echo "[android-prepare] done"