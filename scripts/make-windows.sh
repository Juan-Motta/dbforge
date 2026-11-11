#!/usr/bin/env bash
# Builds release/DBForge-<version>-x64-setup.exe, the Windows installer of the version in
# backend/tauri.conf.json, on Windows (Tauri builds for Windows only there; Git Bash runs this). It
# installs for the current user, without asking for an administrator, and isn't code-signed yet, so
# SmartScreen warns before the first run.
# Usage: scripts/make-windows.sh
# Optional variable:
#   TAURI_SIGNING_PRIVATE_KEY  the updater key (its text or path): also signs the installer for the
#                              app's updater (.sig), which runs it to update
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(node -p "require('./backend/tauri.conf.json').version")"

pnpm tauri build --bundles nsis

mkdir -p release
SETUP="release/DBForge-$VERSION-x64-setup.exe"
cp backend/target/release/bundle/nsis/*-setup.exe "$SETUP"

if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
    scripts/sign-update.sh "$SETUP" "$VERSION"
fi

echo "Built $SETUP"
