import { useEffect, useState, useSyncExternalStore } from 'react'
import {
  applyTheme,
  loadThemePreference,
  resolveTheme,
  saveThemePreference,
  systemIsDark,
  watchSystemTheme,
  type ThemePreference,
} from './theme'

/** The user's theme choice, saved across launches and applied to the page. */
export function useTheme() {
  const [preference, setPreference] = useState(loadThemePreference)
  const isSystemDark = useSyncExternalStore(watchSystemTheme, systemIsDark)
  const theme = resolveTheme(preference, isSystemDark)

  useEffect(() => applyTheme(theme), [theme])

  const choose = (next: ThemePreference) => {
    setPreference(next)
    saveThemePreference(next)
  }

  return { preference, choose }
}
