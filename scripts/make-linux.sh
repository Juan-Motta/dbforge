#!/usr/bin/env bash
# Builds the Linux packages of the version in Cargo.toml, on Linux (x86_64):
# release/DBForge-<version>-amd64.AppImage, which runs on most distributions, and
# release/DBForge-<version>-amd64.deb for Debian and Ubuntu. Both update themselves through the app.
#
# They install what the Tauri app's did, so they upgrade it in place: the .deb is the package `db-forge`
# (Tauri's name for "DBForge"; it also replaces a package named `dbforge`), with the executable at
# /usr/bin/DBForge; the AppImage is one file the updater replaces.
#
# Needs, besides Rust: dpkg-deb, and the libraries GPUI builds against (on Debian or Ubuntu:
# build-essential pkg-config cmake clang libfontconfig-dev libfreetype-dev libwayland-dev libx11-xcb-dev
# libxkbcommon-dev libxkbcommon-x11-dev libvulkan-dev). appimagetool is downloaded when APPIMAGETOOL
# doesn't name one.
# Usage: scripts/make-linux.sh
# Optional variables:
#   APPIMAGETOOL         the appimagetool to run (default: downloaded into target/bundle/tools)
#   UPDATER_PRIVATE_KEY  the updater key (its text or path; UPDATER_PRIVATE_KEY_PASSWORD its password): also
#                        signs both for the app's updater (.sig); the updater installs the kind that runs
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(scripts/app-version.sh)"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
ID="dev.dbforge.app"

cargo build --release --locked -p dbforge
BINARY="target/release/dbforge"
STAGE="target/bundle/linux"
rm -rf "$STAGE"
mkdir -p "$STAGE" release

# The installed tree both packages share: the executable, its menu entry and its icons.
install_tree() { # root
    local root="$1"
    install -Dm755 "$BINARY" "$root/usr/bin/DBForge"
    install -Dm644 "packaging/linux/$ID.desktop" "$root/usr/share/applications/$ID.desktop"
    install -Dm644 packaging/icons/32x32.png "$root/usr/share/icons/hicolor/32x32/apps/$ID.png"
    install -Dm644 packaging/icons/128x128.png "$root/usr/share/icons/hicolor/128x128/apps/$ID.png"
    install -Dm644 "packaging/icons/128x128@2x.png" "$root/usr/share/icons/hicolor/256x256/apps/$ID.png"
    install -Dm644 packaging/icons/icon.png "$root/usr/share/icons/hicolor/512x512/apps/$ID.png"
}

# The .deb.
DEB_ROOT="$STAGE/deb"
install_tree "$DEB_ROOT"
mkdir -p "$DEB_ROOT/DEBIAN"
INSTALLED_SIZE="$(du -sk --exclude=DEBIAN "$DEB_ROOT" | cut -f1)"
sed -e "s/@VERSION@/$VERSION/" -e "s/@INSTALLED_SIZE@/$INSTALLED_SIZE/" packaging/linux/control \
    > "$DEB_ROOT/DEBIAN/control"
DEB="release/DBForge-$VERSION-amd64.deb"
dpkg-deb --build --root-owner-group -Zxz "$DEB_ROOT" "$DEB"

# The AppImage: the same tree in an AppDir, with the entry, icon and AppRun appimagetool expects at its top.
APPDIR="$STAGE/DBForge.AppDir"
install_tree "$APPDIR"
cp "packaging/linux/$ID.desktop" "$APPDIR/$ID.desktop"
cp packaging/icons/icon.png "$APPDIR/$ID.png"
ln -s "$ID.png" "$APPDIR/.DirIcon"
ln -s usr/bin/DBForge "$APPDIR/AppRun"
if [[ -z "${APPIMAGETOOL:-}" ]]; then
    APPIMAGETOOL="target/bundle/tools/appimagetool"
    if [[ ! -x "$APPIMAGETOOL" ]]; then
        mkdir -p "$(dirname "$APPIMAGETOOL")"
        curl --fail --location --silent --show-error --output "$APPIMAGETOOL" "$APPIMAGETOOL_URL"
        chmod +x "$APPIMAGETOOL"
    fi
fi
APPIMAGE="release/DBForge-$VERSION-amd64.AppImage"
rm -f "$APPIMAGE"
# Extract-and-run: CI runners have no FUSE to mount appimagetool's own AppImage.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$APPIMAGETOOL" --no-appstream "$APPDIR" "$APPIMAGE"

# On Linux the update is the package itself: an AppImage replaces itself, a .deb runs dpkg.
if [[ -n "${UPDATER_PRIVATE_KEY:-}" ]]; then
    scripts/sign-update.sh "$APPIMAGE" "$VERSION"
    scripts/sign-update.sh "$DEB" "$VERSION"
fi

echo "Built $APPIMAGE and $DEB"
