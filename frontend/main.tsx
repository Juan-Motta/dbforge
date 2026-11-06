import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { isTauri } from '@tauri-apps/api/core'
import '@fontsource-variable/inter'
import '@fontsource-variable/jetbrains-mono'
import './styles/theme.css'
import './styles/base.css'
import { App } from './app/App'

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
