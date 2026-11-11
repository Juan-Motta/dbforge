#!/usr/bin/env bash
# Prints latest.json, the manifest the app's updater reads from the latest release: the version, its
# notes, and where to download the signed update for each Mac (the universal app serves both).
# Usage: scripts/make-update-manifest.sh <version> <download-url> <signature-file> <notes.md>
set -euo pipefail

VERSION="${1:?usage: scripts/make-update-manifest.sh <version> <download-url> <signature-file> <notes.md>}"
URL="${2:?missing the download URL}"
SIGNATURE="$(cat "${3:?missing the signature file}")"
NOTES="$(cat "${4:?missing the notes file}")"

jq -n \
    --arg version "$VERSION" \
    --arg notes "$NOTES" \
    --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg signature "$SIGNATURE" \
    --arg url "$URL" \
    '{
        version: $version,
        notes: $notes,
        pub_date: $date,
        platforms: {
            "darwin-aarch64": { signature: $signature, url: $url },
            "darwin-x86_64": { signature: $signature, url: $url }
        }
    }'
