#!/usr/bin/env bash
# Signs an update the app's updater installs, writing <file>.sig beside it. The signature is bound to
# <version>, so a manifest can't pair the file with another version number.
# Usage: scripts/sign-update.sh <file> <version>
# Needs TAURI_SIGNING_PRIVATE_KEY: the updater key, its text or the path of its file; and, when the key
# has one, TAURI_SIGNING_PRIVATE_KEY_PASSWORD.
set -euo pipefail

FILE="${1:?usage: scripts/sign-update.sh <file> <version>}"
VERSION="${2:?missing the version}"
: "${TAURI_SIGNING_PRIVATE_KEY:?the updater key is missing}"

rm -f "$FILE.sig"
# The signer reads a key file from TAURI_SIGNING_PRIVATE_KEY_PATH and only the key's text from
# TAURI_SIGNING_PRIVATE_KEY; either way the key stays out of the command line.
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
if [[ -f "$TAURI_SIGNING_PRIVATE_KEY" ]]; then
    env -u TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PATH="$TAURI_SIGNING_PRIVATE_KEY" \
        pnpm tauri signer sign --app-version "$VERSION" "$FILE"
else
    pnpm tauri signer sign --app-version "$VERSION" "$FILE"
fi
