import { messages } from './messages'

/** Backend errors arrive as plain strings; anything else is unexpected. */
export function errorMessage(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return messages[pageLanguage()].unexpectedError
}

/** The interface language, which `applyLanguage` writes on `<html lang>`; English outside a page. */
function pageLanguage(): 'en' | 'es' {
  return typeof document !== 'undefined' && document.documentElement.lang === 'es' ? 'es' : 'en'
}
