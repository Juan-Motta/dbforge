#!/usr/bin/env bash
# Builds the Linux packages of the version in backend/tauri.conf.json, on Linux (Tauri can't
# cross-compile to it): release/DBForge-<version>-amd64.AppImage, which runs on most distributions, and
# release/DBForge-<version>-amd64.deb for Debian and Ubuntu. Both update themselves through the app.
# Needs Tauri's Linux packages: libwebkit2gtk-4.1-dev, libayatana-appindicator3-dev, librsvg2-dev,
# libssl-dev, build-essential, file and patchelf.
# Usage: scripts/make-linux.sh
# Optional variable:
#   TAURI_SIGNING_PRIVATE_KEY  the updater key (its text or path): also signs both for the app's
#                              updater (.sig); the updater installs the kind of package that is running
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(node -p "require('./backend/tauri.conf.json').version")"

pnpm tauri build --bundles appimage,deb
BUNDLE="backend/target/release/bundle"

mkdir -p release
APPIMAGE="release/DBForge-$VERSION-amd64.AppImage"
DEB="release/DBForge-$VERSION-amd64.deb"
cp "$BUNDLE"/appimage/*.AppImage "$APPIMAGE"
cp "$BUNDLE"/deb/*.deb "$DEB"

# On Linux the update is the package itself: an AppImage replaces itself, a .deb runs dpkg.
if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
    scripts/sign-update.sh "$APPIMAGE" "$VERSION"
    scripts/sign-update.sh "$DEB" "$VERSION"
fi

echo "Built $APPIMAGE and $DEB"
