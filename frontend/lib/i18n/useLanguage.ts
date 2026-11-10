import { createContext, useContext } from 'react'
import type { Language } from './language'
import type { Message } from './messages'

export interface LanguageState {
  language: Language
  choose: (language: Language) => void
}

/** Provided by `LanguageProvider`; English until it is mounted. */
export const LanguageContext = createContext<LanguageState>({ language: 'en', choose: () => {} })

export function useLanguage(): LanguageState {
  return useContext(LanguageContext)
}

/** A feature's texts in the interface language. */
export function useMessages<T extends Record<string, Message>>(messages: { en: T; es: T }): T {
  return messages[useLanguage().language]
}
