import { loadExportFolder, saveExportFolder } from '../dump/exportFolder'
import { loadThemePreference, saveThemePreference, type ThemePreference } from '../settings/theme'
import { loadLanguage, saveLanguage, type Language } from '../../lib/i18n/language'
import type { Preferences } from './api'

/** The preferences this window keeps, for a backup. */
export function currentPreferences(): Preferences {
  const folder = loadExportFolder()
  return {
    theme: loadThemePreference(),
    language: loadLanguage(),
    ...(folder ? { exportFolder: folder } : {}),
  }
}

const THEMES: ThemePreference[] = ['dark', 'light', 'system']
const LANGUAGES: Language[] = ['en', 'es']

/** A backup's preferences, keeping only values this version understands. */
export function knownPreferences(preferences: Preferences): {
  theme?: ThemePreference
  language?: Language
  exportFolder?: string
} {
  const theme = THEMES.find((value) => value === preferences.theme)
  const language = LANGUAGES.find((value) => value === preferences.language)
  const exportFolder = preferences.exportFolder?.trim() || undefined
  return { theme, language, exportFolder }
}

/** Saves a backup's known preferences; the caller applies the theme and language to the window. */
export function savePreferences(preferences: Preferences) {
  const known = knownPreferences(preferences)
  if (known.theme) saveThemePreference(known.theme)
  if (known.language) saveLanguage(known.language)
  if (known.exportFolder) saveExportFolder(known.exportFolder)
  return known
}

/** `dbforge-backup-2026-11-10.json`. */
export function backupFileName(at: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  return `dbforge-backup-${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}.json`
}
