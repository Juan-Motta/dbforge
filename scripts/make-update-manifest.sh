#!/usr/bin/env bash
# Prints latest.json, the manifest the app's updater reads from the latest release: the version, its
# notes, and where to download the signed update for each platform. Each update is given as its
# platforms (comma-separated, as the updater names them), its download URL and its signature file;
# the universal Mac app serves darwin-aarch64 and darwin-x86_64 at once.
# Usage: scripts/make-update-manifest.sh <version> <notes.md> <platforms> <url> <signature-file> [...]
# e.g.   scripts/make-update-manifest.sh 0.3.0 notes.md \
#          darwin-aarch64,darwin-x86_64 https://…/DBForge-0.3.0.app.tar.gz DBForge-0.3.0.app.tar.gz.sig \
#          linux-x86_64 https://…/DBForge-0.3.0-amd64.AppImage DBForge-0.3.0-amd64.AppImage.sig
set -euo pipefail

USAGE="usage: scripts/make-update-manifest.sh <version> <notes.md> <platforms> <url> <signature-file> [...]"
VERSION="${1:?$USAGE}"
NOTES="$(cat "${2:?$USAGE}")"
shift 2
if (($# == 0 || $# % 3 != 0)); then
    echo "$USAGE" >&2
    exit 1
fi

PLATFORMS='{}'
while (($# > 0)); do
    SIGNATURE="$(cat "$3")"
    IFS=',' read -ra NAMES <<<"$1"
    for name in "${NAMES[@]}"; do
        PLATFORMS="$(jq --arg name "$name" --arg url "$2" --arg signature "$SIGNATURE" \
            '. + {($name): {signature: $signature, url: $url}}' <<<"$PLATFORMS")"
    done
    shift 3
done

jq -n \
    --arg version "$VERSION" \
    --arg notes "$NOTES" \
    --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --argjson platforms "$PLATFORMS" \
    '{version: $version, notes: $notes, pub_date: $date, platforms: $platforms}'
