import { afterEach, describe, expect, it, vi } from 'vitest'
import { loadLanguage, saveLanguage, systemLanguage } from './language'

describe('systemLanguage', () => {
  it('speaks Spanish for any Spanish locale and English otherwise', () => {
    expect(systemLanguage(['es-CO', 'en-US'])).toBe('es')
    expect(systemLanguage(['ES'])).toBe('es')
    expect(systemLanguage(['en-GB', 'es'])).toBe('en')
    expect(systemLanguage(['pt-BR'])).toBe('en')
    expect(systemLanguage([])).toBe('en')
  })
})

describe('loadLanguage', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('keeps a saved choice and otherwise follows the system', () => {
    const store = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => store.set(key, value),
    })
    vi.stubGlobal('navigator', { languages: ['es-MX'] })
    expect(loadLanguage()).toBe('es')
    saveLanguage('en')
    expect(loadLanguage()).toBe('en')
    store.set('dbforge.language', 'fr')
    expect(loadLanguage()).toBe('es')
  })
})
