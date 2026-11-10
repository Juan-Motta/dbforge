import { useEffect, useState, type ReactNode } from 'react'
import { applyLanguage, loadLanguage, saveLanguage, type Language } from './language'
import { LanguageContext } from './useLanguage'

/** Holds the interface language, saved across launches. */
export function LanguageProvider({ children }: { children: ReactNode }) {
  const [language, setLanguage] = useState(loadLanguage)

  useEffect(() => applyLanguage(language), [language])

  const choose = (next: Language) => {
    setLanguage(next)
    saveLanguage(next)
  }

  return <LanguageContext value={{ language, choose }}>{children}</LanguageContext>
}
