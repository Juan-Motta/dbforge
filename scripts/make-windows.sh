#!/usr/bin/env bash
# Builds release/DBForge-<version>-x64-setup.exe, the Windows installer of the version in Cargo.toml, on
# Windows (Git Bash runs this): DBForge.exe built with cargo, packed by makensis with
# packaging/windows/installer.nsi. It installs for the current user, without asking for an administrator,
# where the Tauri app's installer did, so it upgrades an installed copy in place. It isn't code-signed yet,
# so SmartScreen warns before the first run.
# Needs NSIS (makensis on the PATH, or in its default folder).
# Usage: scripts/make-windows.sh
# Optional variable:
#   UPDATER_PRIVATE_KEY  the updater key (its text or path; UPDATER_PRIVATE_KEY_PASSWORD its password): also
#                        signs the installer for the app's updater (.sig), which runs it to update
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(scripts/app-version.sh)"
# Windows' file version takes four numbers: 1.2.3-rc.1 -> 1.2.3.0.
NUMERIC="${VERSION%%[-+]*}.0"

MAKENSIS="$(command -v makensis || true)"
for candidate in "/c/Program Files (x86)/NSIS/makensis.exe" "/c/Program Files/NSIS/makensis.exe"; do
    if [[ -z "$MAKENSIS" && -x "$candidate" ]]; then
        MAKENSIS="$candidate"
    fi
done
if [[ -z "$MAKENSIS" ]]; then
    echo "makensis is missing: install NSIS (choco install nsis)" >&2
    exit 1
fi

cargo build --release --locked -p dbforge

mkdir -p release
SETUP="release/DBForge-$VERSION-x64-setup.exe"
rm -f "$SETUP"
# makensis wants Windows paths for what it packs and writes.
winpath() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
"$MAKENSIS" -V3 -INPUTCHARSET UTF8 \
    -DVERSION="$VERSION" \
    -DVERSIONWITHBUILD="$NUMERIC" \
    -DSOURCE="$(winpath "$PWD/target/release/dbforge.exe")" \
    -DICON="$(winpath "$PWD/packaging/icons/icon.ico")" \
    -DOUTFILE="$(winpath "$PWD/$SETUP")" \
    "$(winpath "$PWD/packaging/windows/installer.nsi")"

if [[ -n "${UPDATER_PRIVATE_KEY:-}" ]]; then
    scripts/sign-update.sh "$SETUP" "$VERSION"
fi

echo "Built $SETUP"
