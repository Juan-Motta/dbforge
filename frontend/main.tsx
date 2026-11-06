import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { isTauri } from '@tauri-apps/api/core'
import '@fontsource-variable/inter'
import '@fontsource-variable/jetbrains-mono'
import './styles/theme.css'
import './styles/base.css'
import { App } from './app/App'
import { applyTheme, loadThemePreference, resolveTheme, systemIsDark } from './features/settings/theme'

// Apply the saved theme before the first paint so the window doesn't flash dark.
applyTheme(resolveTheme(loadThemePreference(), systemIsDark()))

// Outside the desktop app (plain `pnpm dev`), fake the backend. Never bundled in production.
if (import.meta.env.DEV && !isTauri()) {
  const { installMockBackend } = await import('./dev/mockBackend')
  installMockBackend()
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
