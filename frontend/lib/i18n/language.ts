export type Language = 'en' | 'es'

/** Each language named in itself, as the settings list them. */
export const LANGUAGES: { value: Language; label: string }[] = [
  { value: 'en', label: 'English' },
  { value: 'es', label: 'Español' },
]

const STORAGE_KEY = 'dbforge.language'

/** The system's language when DBForge speaks it, otherwise English. */
export function systemLanguage(locales: readonly string[]): Language {
  return locales[0]?.toLowerCase().startsWith('es') ? 'es' : 'en'
}

/** The saved choice; the system's language when nothing (or something unknown) was saved. */
export function loadLanguage(): Language {
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    if (saved === 'en' || saved === 'es') return saved
  } catch {
    // Storage can be unavailable; fall back to the system's language.
  }
  return systemLanguage(navigator.languages)
}

export function saveLanguage(language: Language) {
  try {
    localStorage.setItem(STORAGE_KEY, language)
  } catch {
    // Storage can be unavailable; the choice still applies until the app closes.
  }
}

/** Screen readers and the spell checker read the page in this language. */
export function applyLanguage(language: Language) {
  document.documentElement.lang = language
}
