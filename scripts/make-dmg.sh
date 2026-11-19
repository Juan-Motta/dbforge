#!/usr/bin/env bash
# Builds release/DBForge-<version>.dmg, the version being Cargo.toml's: the installer window with the app
# (scripts/make-app.sh), an Applications shortcut to drag it onto, and the background with the arrow and
# the INSERT INTO statement. Needs dmgbuild (pip install dmgbuild==1.6.7).
# Usage: scripts/make-dmg.sh
# Optional variables:
#   TARGET                  a Rust target, e.g. universal-apple-darwin for Apple silicon and Intel at once
#   APPLE_SIGNING_IDENTITY  "Developer ID Application: Name (TEAMID)": signs the app and the DMG
#   NOTARY_KEY_PATH         App Store Connect API key (.p8): notarizes the DMG and staples its ticket
#   NOTARY_KEY_ID           its key ID
#   NOTARY_ISSUER_ID        its issuer ID
#   UPDATER_PRIVATE_KEY     the updater key (its text or path; UPDATER_PRIVATE_KEY_PASSWORD its password):
#                           also writes the update the app installs, release/DBForge-<version>.app.tar.gz,
#                           and its signature (.sig), see scripts/sign-update.sh
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(scripts/app-version.sh)"
TARGET="${TARGET:-}"
DMG="release/DBForge-$VERSION.dmg"
DMGBUILD="${DMGBUILD:-dmgbuild}"
if ! command -v "$DMGBUILD" >/dev/null; then
    echo "dmgbuild is missing: pip install dmgbuild==1.6.7 (or set DMGBUILD to its path)" >&2
    exit 1
fi

APP="$(TARGET="$TARGET" scripts/make-app.sh)"

# dmgbuild writes the window layout (background, icon positions, volume icon) without driving Finder,
# so it also works on CI runners. Layout: scripts/dmg-settings.py; background: make-dmg-background.swift.
mkdir -p release
rm -f "$DMG"
"$DMGBUILD" -s scripts/dmg-settings.py -D app="$APP" "DBForge $VERSION" "$DMG"

if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
    codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG"
fi

if [[ -n "${NOTARY_KEY_PATH:-}" ]]; then
    : "${NOTARY_KEY_ID:?NOTARY_KEY_ID is required with NOTARY_KEY_PATH}"
    : "${NOTARY_ISSUER_ID:?NOTARY_ISSUER_ID is required with NOTARY_KEY_PATH}"
    AUTH=(--key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")
    RESULT="$(xcrun notarytool submit "$DMG" "${AUTH[@]}" --wait --output-format json)"
    echo "$RESULT"
    if [[ "$(plutil -extract status raw - <<<"$RESULT")" != "Accepted" ]]; then
        # Print Apple's reasons before failing.
        xcrun notarytool log "$(plutil -extract id raw - <<<"$RESULT")" "${AUTH[@]}" || true
        echo "Notarization was not accepted" >&2
        exit 1
    fi
    # Stapling the ticket lets Gatekeeper accept it offline too.
    xcrun stapler staple "$DMG"
fi

if [[ -n "${UPDATER_PRIVATE_KEY:-}" ]]; then
    # The update is the app in the DMG. Notarizing the DMG notarized it too, so its ticket can be stapled
    # to it, and the updated app opens offline like a fresh install.
    if [[ -n "${NOTARY_KEY_PATH:-}" ]]; then
        xcrun stapler staple "$APP"
    fi
    UPDATE="release/DBForge-$VERSION.app.tar.gz"
    rm -f "$UPDATE"
    # No AppleDouble (._) files in the archive.
    COPYFILE_DISABLE=1 tar -czf "$UPDATE" -C "$(dirname "$APP")" "$(basename "$APP")"
    scripts/sign-update.sh "$UPDATE" "$VERSION"
    echo "Built $UPDATE"
fi

echo "Built $DMG"
