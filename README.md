# DBForge

A lightweight, cross-platform database viewer built with Tauri 2, React 19 and TypeScript.

## Commands

| Command | What it does |
| --- | --- |
| `pnpm dev` | Runs the UI in the browser at http://localhost:5173 |
| `pnpm tauri dev` | Runs the desktop app |
| `pnpm test` | Runs unit tests (Vitest) |
| `pnpm lint` | Lints with oxlint |
| `pnpm build` | Type-checks and builds the UI |

## Releases

### A DMG on your Mac

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

### Publishing a version

1. Set the version in `backend/tauri.conf.json` (and `package.json` and `backend/Cargo.toml` to match) and commit it.
2. Tag it and push the tag:

   ```bash
   git tag v0.2.0 && git push origin v0.2.0
   ```

The `Release` workflow then tests, builds a signed and notarized universal DMG with `scripts/make-dmg.sh`, and publishes it as a
GitHub Release with notes made from the commits since the last release (`scripts/release-notes.sh`).
A tag that doesn't match the version in `tauri.conf.json` fails before building.

It needs these repository secrets, the same as ColiUI's:

| Secret | What it holds |
| --- | --- |
| `MACOS_CERTIFICATE_P12_BASE64` | The Developer ID Application certificate, exported as .p12, in base64 (`base64 -i cert.p12`) |
| `MACOS_CERTIFICATE_PASSWORD` | The password the .p12 was exported with |
| `APP_STORE_CONNECT_API_KEY_P8` | The text of the App Store Connect API key's `.p8` file |
| `APP_STORE_CONNECT_KEY_ID` | That key's ID |
| `APP_STORE_CONNECT_ISSUER_ID` | The issuer ID shown above the keys in App Store Connect |
| `TAURI_SIGNING_PRIVATE_KEY` | The text of the updater's private key, `~/.tauri/dbforge.key` |

### Updates

DBForge looks for a new version once a day (Settings › About can turn that off) and from
DBForge › Check for Updates…, reading `latest.json` from the latest GitHub Release. Each release
carries that manifest and `DBForge-<version>.app.tar.gz`, the notarized app, signed with the
updater's own key. The app checks that signature against the public key in `backend/tauri.conf.json`,
and that it was made for the version the manifest announces, before installing anything.

The private key was made with `pnpm tauri signer generate -w ~/.tauri/dbforge.key` and has no
password. Keep a copy somewhere safe: without it, no installed copy of DBForge can be updated again,
and a new key means people reinstall by hand once. `scripts/make-dmg.sh` signs the update when
`TAURI_SIGNING_PRIVATE_KEY` holds the key's text or path.

## Structure

```
frontend/
  app/          App composition (layout of the shell)
  components/   Reusable, feature-agnostic UI (Button, Icon, Badge, EngineLogo)
  domain/       Core types shared across features (Connection, Engine, TableRef)
  features/     One folder per feature, with its components, logic and styles
    explorer/   Database explorer tree
    shell/      Title bar, activity rail, status bar
  styles/       Design tokens (theme.css) and global base styles
backend/        Rust backend (Tauri)
```

Conventions:

- Styles use CSS Modules next to each component; colors come only from the tokens in `styles/theme.css`.
- Pure logic (for example `features/explorer/tree.ts`) lives outside components and is unit tested.
- A feature imports from `components/` and `domain/`, never from another feature.
