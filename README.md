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
pnpm tauri build --bundles dmg
```

The DMG lands in `backend/target/release/bundle/dmg/` (for example `DBForge_0.1.0_aarch64.dmg`), built for
this Mac's architecture. Without more, it is only signed ad hoc, which runs on your Mac but which Gatekeeper
blocks on others. It is signed with the Developer ID certificate in your keychain when you name it, and
notarized when the App Store Connect key is given too:

```bash
APPLE_SIGNING_IDENTITY="Developer ID Application: <name> (<team id>)" \
APPLE_API_KEY=<key id> APPLE_API_ISSUER=<issuer id> APPLE_API_KEY_PATH=<path to AuthKey.p8> \
pnpm tauri build --target universal-apple-darwin --bundles dmg
```

`--target universal-apple-darwin` builds for Apple silicon and Intel at once
(`rustup target add aarch64-apple-darwin x86_64-apple-darwin` first).

### Publishing a version

1. Set the version in `backend/tauri.conf.json` (and `package.json` and `backend/Cargo.toml` to match) and commit it.
2. Tag it and push the tag:

   ```bash
   git tag v0.2.0 && git push origin v0.2.0
   ```

The `Release` workflow then tests, builds a signed and notarized universal DMG, and publishes it as a
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
