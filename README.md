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

## Structure

```
frontend/
  app/          App composition (layout of the shell)
  components/   Reusable, feature-agnostic UI (Button, Icon, Badge, EngineLogo)
  domain/       Core types shared across features (Connection, Engine)
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
