#!/usr/bin/env bash
# Builds DBForge.app, the macOS app of the version in Cargo.toml, into target/bundle/macos/ and prints its
# path. The bundle is the Tauri app's: named DBForge.app, executable Contents/MacOS/DBForge, identifier
# dev.dbforge.app (packaging/macos/Info.plist), so a DMG or an update replaces an installed copy in place.
# Usage: scripts/make-app.sh
# Optional variables:
#   TARGET                  a Rust target: universal-apple-darwin builds Apple silicon and Intel into one
#                           executable (needs both rustup targets); default: this Mac's
#   APPLE_SIGNING_IDENTITY  "Developer ID Application: Name (TEAMID)": signs it with the hardened runtime,
#                           as notarization requires; without it the app is signed ad hoc (runs on this Mac)
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(scripts/app-version.sh)"
TARGET="${TARGET:-}"
# GPUI's oldest macOS; the executable and Info.plist say the same.
MINIMUM_SYSTEM_VERSION="10.15.7"
export MACOSX_DEPLOYMENT_TARGET="$MINIMUM_SYSTEM_VERSION"

build() { # [rust target]
    cargo build --release --locked -p dbforge ${1:+--target "$1"} >&2
}

BUNDLE_DIR="target/bundle/macos"
mkdir -p "$BUNDLE_DIR"
EXECUTABLE="$BUNDLE_DIR/DBForge-executable"
case "$TARGET" in
    universal-apple-darwin)
        build aarch64-apple-darwin
        build x86_64-apple-darwin
        lipo -create -output "$EXECUTABLE" \
            target/aarch64-apple-darwin/release/dbforge target/x86_64-apple-darwin/release/dbforge
        ;;
    "")
        build
        cp target/release/dbforge "$EXECUTABLE"
        ;;
    *)
        build "$TARGET"
        cp "target/$TARGET/release/dbforge" "$EXECUTABLE"
        ;;
esac

APP="$BUNDLE_DIR/DBForge.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
mv "$EXECUTABLE" "$APP/Contents/MacOS/DBForge"
cp packaging/icons/icon.icns "$APP/Contents/Resources/icon.icns"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@MINIMUM_SYSTEM_VERSION@/$MINIMUM_SYSTEM_VERSION/g" \
    packaging/macos/Info.plist > "$APP/Contents/Info.plist"
plutil -lint -s "$APP/Contents/Info.plist"

if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
    # The hardened runtime, with no entitlements, as Tauri signed it.
    codesign --force --timestamp --options runtime --sign "$APPLE_SIGNING_IDENTITY" "$APP" >&2
else
    codesign --force --sign - "$APP" >&2
fi
codesign --verify --strict "$APP" >&2

echo "$APP"
