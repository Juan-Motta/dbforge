# Releasing DBForge

How a version of the desktop app (`crates/app`, drawn with GPUI) is built for macOS, Linux and Windows,
signed, notarized and published, and how installed copies update themselves.

The version lives in one place: `version` under `[workspace.package]` in `Cargo.toml`
(`scripts/app-version.sh` prints it). The app, its packages, the updater's signatures and the release tag
all take it from there.

## The app on your Mac

```bash
scripts/make-app.sh
```

It builds `target/bundle/macos/DBForge.app` for this Mac's architecture (`TARGET=universal-apple-darwin`
builds for Apple silicon and Intel at once, after `rustup target add aarch64-apple-darwin
x86_64-apple-darwin`). The bundle is laid out by hand: `packaging/macos/Info.plist` (name `DBForge`,
identifier `dev.dbforge.app`, macOS 10.15.7 or later) and `packaging/icons/icon.icns`. Without more it is
signed ad hoc; `APPLE_SIGNING_IDENTITY` signs it with a Developer ID certificate in your keychain and the
hardened runtime, as notarization requires.

## A DMG on your Mac

```bash
python3 -m pip install dmgbuild==1.6.7   # once
scripts/make-dmg.sh
```

It writes `release/DBForge-<version>.dmg` with the app of `scripts/make-app.sh`. The installer window shows
the app, an Applications shortcut, and between them an arrow and an `INSERT INTO Applications` statement,
drawn by `scripts/make-dmg-background.swift` into `packaging/macos/dmg-background.tiff` (run it with `swift`
after changing it); `scripts/dmg-settings.py` lays it out.

Without more, the DMG is only signed ad hoc, which runs on your Mac but which Gatekeeper blocks on others.
`APPLE_SIGNING_IDENTITY` signs the app and the DMG with a Developer ID certificate, and the `NOTARY_KEY_PATH`,
`NOTARY_KEY_ID` and `NOTARY_ISSUER_ID` of an App Store Connect API key notarize it (see the script's header).

## Linux packages

Made in CI (or on an x86_64 Debian or Ubuntu machine) with:

```bash
sudo apt-get install -y build-essential pkg-config cmake clang file libfontconfig-dev libfreetype-dev \
  libwayland-dev libx11-xcb-dev libxkbcommon-dev libxkbcommon-x11-dev libvulkan-dev   # once
scripts/make-linux.sh
```

It writes `release/DBForge-<version>-amd64.AppImage` (made with appimagetool, downloaded unless `APPIMAGETOOL`
names one), which runs on most distributions, and `release/DBForge-<version>-amd64.deb` (made with
`dpkg-deb`, package `db-forge`, executable `/usr/bin/DBForge`) for Debian and Ubuntu. To try them without
publishing, run the `Release` workflow by hand (Actions › Release › Run workflow): it builds and keeps the
packages as the run's artifacts, and builds the DMG too only when asked.

## Windows installer

Made in CI (or on Windows, from Git Bash, with NSIS installed) with `scripts/make-windows.sh`. It writes
`release/DBForge-<version>-x64-setup.exe`, built by `makensis` from `packaging/windows/installer.nsi`: it
installs for the current user (no administrator needed) into `%LOCALAPPDATA%\DBForge`, where the former Tauri app's
installer did, with the same uninstall entry, so it upgrades an installed copy in place. It isn't
code-signed: SmartScreen warns before the first run, until a code-signing certificate (or Azure Trusted
Signing) is added to the build.

## Publishing a version

1. Set the version in `Cargo.toml` (`[workspace.package]`), refresh `Cargo.lock` (`cargo check`) and commit both.
2. Tag it and push the tag:

   ```bash
   git tag v0.4.0 && git push origin v0.4.0
   ```

The `Release` workflow then tests, builds a signed and notarized universal DMG with `scripts/make-dmg.sh`,
the Linux packages with `scripts/make-linux.sh` and the Windows installer with `scripts/make-windows.sh` at
the same time, and publishes them as one GitHub Release with notes made from the commits since the last
release (`scripts/release-notes.sh`). If any build fails, nothing is published. A tag that doesn't match the
version in `Cargo.toml` fails before building.

It needs these repository secrets:

| Secret | What it holds |
| --- | --- |
| `MACOS_CERTIFICATE_P12_BASE64` | The Developer ID Application certificate, exported as .p12, in base64 (`base64 -i cert.p12`) |
| `MACOS_CERTIFICATE_PASSWORD` | The password the .p12 was exported with |
| `APP_STORE_CONNECT_API_KEY_P8` | The text of the App Store Connect API key's `.p8` file |
| `APP_STORE_CONNECT_KEY_ID` | That key's ID |
| `APP_STORE_CONNECT_ISSUER_ID` | The issuer ID shown above the keys in App Store Connect |
| `TAURI_SIGNING_PRIVATE_KEY` | The text of the updater's private key, `~/.tauri/dbforge.key` (every platform) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Its password, if it has one (it has none) |

The updater key's secrets keep the names they had when the former Tauri app's tooling signed the updates; the workflow hands them to
the scripts as `UPDATER_PRIVATE_KEY` and `UPDATER_PRIVATE_KEY_PASSWORD`. On Linux and Windows the key only
reaches a signing step of its own, after the build, so build scripts and packaging tools never see it; on macOS
`scripts/make-dmg.sh` still builds and signs in one step, as before.

## Updates

DBForge looks for a new version once a day (Settings › About can turn that off) and from DBForge › Check for
Updates… or Settings › About › Check for Updates…, reading `latest.json` from the latest GitHub Release.
Each release carries that manifest, `DBForge-<version>.app.tar.gz` (the notarized Mac app), the AppImage,
the .deb and the Windows installer, each signed with the updater's own key (`scripts/sign-update.sh`);
`scripts/make-update-manifest.sh` lists them per platform (`darwin-*`, `linux-x86_64-appimage`,
`linux-x86_64-deb`, `windows-x86_64-nsis`).

The app (`crates/app/src/updates`) checks the signature against the public key in `packaging/updater.pub`,
and that it was made for the version the manifest announces, before installing anything. Then it installs
the kind of package that is running: on macOS it swaps the app bundle for the archive's; an AppImage
replaces its own file; a .deb runs `pkexec dpkg -i`, so Linux asks for an administrator's password; on
Windows the installer runs passive (`/P /UPDATE /R`) and starts the new version. The new version starts by
itself.

Copies installed by the former Tauri app (0.3.x) update from the same manifest with Tauri's updater, which reads
the same names, formats and signatures, so they move to the GPUI app like any other update. Keep it so:
`scripts/make-update-manifest.sh`'s shape, the artifact names, the bundle identifier `dev.dbforge.app`, the
`DBForge` executable and the installers' locations must not change.

The signer is `dbforge-release` (`crates/release`), a small Rust tool over the `minisign` crate that writes
the format `tauri signer sign` wrote: the base64 of a minisign signature file whose trusted comment names
the file and the version. `scripts/sign-update.sh` signs with it, then verifies the signature against
`packaging/updater.pub`, so a release never carries an update installed copies would refuse.

The private key was made, back when Tauri built DBForge, with `tauri signer generate -w ~/.tauri/dbforge.key`
and has no password; `dbforge-release` reads that same key. Keep a
copy somewhere safe: without it, no installed copy of DBForge can be updated again, and a new key means
people reinstall by hand once. The packaging scripts sign the update when `UPDATER_PRIVATE_KEY` holds the
key's text or path.

To try the updater against a local manifest, run a development build with `DBFORGE_UPDATE_ENDPOINT` (the
manifest's URL), `DBFORGE_UPDATE_PUBKEY` (a test key's public half, as `dbforge-release public-key` prints
it) and, so nothing installed is touched, `DBFORGE_UPDATE_INSTALL_PATH` (a stand-in for the app, such as a
copy of `DBForge.app` in a temporary folder). Release builds ignore these variables.
