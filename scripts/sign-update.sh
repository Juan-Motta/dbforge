#!/usr/bin/env bash
# Signs an update the app's updater installs, writing <file>.sig beside it, then checks the signature with
# the public key the app ships (packaging/updater.pub), so a release never carries an update no installed
# copy would accept. The signature is bound to <version>, so a manifest can't pair the file with another
# version number. Its format is the one `tauri signer` wrote, which DBForge 0.3.x copies verify too.
# Usage: scripts/sign-update.sh <file> <version>
# Needs UPDATER_PRIVATE_KEY: the updater key, its text or the path of its file; and, when the key has one,
# UPDATER_PRIVATE_KEY_PASSWORD. Both stay in the environment, out of every command line.
# Optional: UPDATER_PUBLIC_KEY_FILE, the public key to check against (default packaging/updater.pub), for a
# test key.
set -euo pipefail

FILE="${1:?usage: scripts/sign-update.sh <file> <version>}"
VERSION="${2:?missing the version}"
: "${UPDATER_PRIVATE_KEY:?the updater key is missing}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PUBLIC_KEY="${UPDATER_PUBLIC_KEY_FILE:-$ROOT/packaging/updater.pub}"

rm -f "$FILE.sig"
cargo build --quiet --release --locked --manifest-path "$ROOT/Cargo.toml" -p dbforge-release
SIGNER="$ROOT/target/release/dbforge-release"
"$SIGNER" sign "$FILE" "$VERSION"
"$SIGNER" verify "$FILE" "$VERSION" "$PUBLIC_KEY"
