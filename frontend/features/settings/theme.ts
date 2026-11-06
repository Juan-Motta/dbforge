export type Theme = 'dark' | 'light'
export type ThemePreference = Theme | 'system'

const STORAGE_KEY = 'dbforge.theme'
const SYSTEM_DARK = '(prefers-color-scheme: dark)'

export function resolveTheme(preference: ThemePreference, systemIsDark: boolean): Theme {
  if (preference !== 'system') return preference
  return systemIsDark ? 'dark' : 'light'
}

/** The saved preference; dark when nothing (or something unknown) was saved. */
export function loadThemePreference(): ThemePreference {
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    return saved === 'light' || saved === 'system' ? saved : 'dark'
  } catch {
    return 'dark'
  }
}

export function saveThemePreference(preference: ThemePreference) {
  try {
    localStorage.setItem(STORAGE_KEY, preference)
  } catch {
    // Storage can be unavailable; the choice still applies until the app closes.
  }
}

export function systemIsDark(): boolean {
  return window.matchMedia(SYSTEM_DARK).matches
}

/** Calls `onChange` when the OS switches between light and dark; returns an unsubscribe. */
export function watchSystemTheme(onChange: () => void): () => void {
  const query = window.matchMedia(SYSTEM_DARK)
  query.addEventListener('change', onChange)
  return () => query.removeEventListener('change', onChange)
}

/** The tokens in styles/theme.css switch on this attribute. */
export function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme
}
