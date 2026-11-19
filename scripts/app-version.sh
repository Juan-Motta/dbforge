#!/usr/bin/env bash
# Prints DBForge's version: `version` under [workspace.package] in Cargo.toml, the one place it is set.
# The app, its packages, the updater's signatures and the release tag all take it from there.
# Usage: scripts/app-version.sh
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(awk '
    /^\[/ { inside = ($0 == "[workspace.package]") }
    inside && $1 == "version" && $2 == "=" { gsub(/"/, "", $3); print $3; exit }
' Cargo.toml)"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]]; then
    echo "No valid version under [workspace.package] in Cargo.toml (found '$VERSION')" >&2
    exit 1
fi
echo "$VERSION"
