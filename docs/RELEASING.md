# Releasing DBForge

How a version is built for macOS and Linux, signed, notarized and published, and how installed copies
update themselves.

## A DMG on your Mac

```bash
python3 -m pip install dmgbuild==1.6.7   # once
scripts/make-dmg.sh
```

It writes `release/DBForge-<version>.dmg`, the version being `backend/tauri.conf.json`'s, built for this Mac's
architecture (`TARGET=universal-apple-darwin` builds for Apple silicon and Intel at once, after
`rustup target add aarch64-apple-darwin x86_64-apple-darwin`). The installer window shows the app, an
Applications shortcut, and between them an arrow and an `INSERT INTO Applications` statement, drawn by
`scripts/make-dmg-background.swift` into `backend/dmg/background.tiff` (run it with `swift` after changing it).

Without more, the DMG is only signed ad hoc, which runs on your Mac but which Gatekeeper blocks on others.
`APPLE_SIGNING_IDENTITY` signs it with a Developer ID certificate in your keychain, and the `NOTARY_KEY_PATH`,
`NOTARY_KEY_ID` and `NOTARY_ISSUER_ID` of an App Store Connect API key notarize it (see the script's header).

## Linux packages

Tauri builds for Linux only on Linux, so they are made in CI (or on a Linux machine) with:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libssl-dev build-essential file patchelf   # once, on Debian or Ubuntu
scripts/make-linux.sh
```

It writes `release/DBForge-<version>-amd64.AppImage`, which runs on most distributions, and
`release/DBForge-<version>-amd64.deb` for Debian and Ubuntu. With `TAURI_SIGNING_PRIVATE_KEY` it also
signs both for the updater, which installs the kind of package that is running. To try the Linux build without
publishing, run the `Release` workflow by hand (Actions › Release › Run workflow): it builds and keeps the
packages as the run's artifacts, and builds the DMG too only when asked.

## Publishing a version

1. Set the version in `backend/tauri.conf.json` (and `package.json` and `backend/Cargo.toml` to match) and commit it.
2. Tag it and push the tag:

   ```bash
   git tag v0.2.0 && git push origin v0.2.0
   ```

The `Release` workflow then tests, builds a signed and notarized universal DMG with `scripts/make-dmg.sh` and
the Linux packages with `scripts/make-linux.sh` at the same time, and publishes them as one GitHub Release with
notes made from the commits since the last release (`scripts/release-notes.sh`). If either build fails,
nothing is published.
A tag that doesn't match the version in `tauri.conf.json` fails before building.

It needs these repository secrets, the same as ColiUI's:

| Secret | What it holds |
| --- | --- |
| `MACOS_CERTIFICATE_P12_BASE64` | The Developer ID Application certificate, exported as .p12, in base64 (`base64 -i cert.p12`) |
| `MACOS_CERTIFICATE_PASSWORD` | The password the .p12 was exported with |
| `APP_STORE_CONNECT_API_KEY_P8` | The text of the App Store Connect API key's `.p8` file |
| `APP_STORE_CONNECT_KEY_ID` | That key's ID |
| `APP_STORE_CONNECT_ISSUER_ID` | The issuer ID shown above the keys in App Store Connect |
| `TAURI_SIGNING_PRIVATE_KEY` | The text of the updater's private key, `~/.tauri/dbforge.key` (both platforms) |

## Updates

DBForge looks for a new version once a day (Settings › About can turn that off) and from
DBForge › Check for Updates…, reading `latest.json` from the latest GitHub Release. Each release
carries that manifest, `DBForge-<version>.app.tar.gz` (the notarized Mac app), the AppImage and the .deb,
each signed with the updater's own key (`scripts/sign-update.sh`); `scripts/make-update-manifest.sh` lists
them per platform (`darwin-*`, `linux-x86_64-appimage`, `linux-x86_64-deb`). A .deb update runs `dpkg -i`,
so Linux asks for an administrator's password to install it. The app checks that signature against the public key in `backend/tauri.conf.json`,
and that it was made for the version the manifest announces, before installing anything.

The private key was made with `pnpm tauri signer generate -w ~/.tauri/dbforge.key` and has no
password. Keep a copy somewhere safe: without it, no installed copy of DBForge can be updated again,
and a new key means people reinstall by hand once. `scripts/make-dmg.sh` and `scripts/make-linux.sh` sign the
update when `TAURI_SIGNING_PRIVATE_KEY` holds the key's text or path.
