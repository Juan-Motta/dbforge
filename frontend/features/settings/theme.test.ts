import { afterEach, describe, expect, it, vi } from 'vitest'
import { loadThemePreference, resolveTheme, saveThemePreference } from './theme'

describe('resolveTheme', () => {
  it('uses an explicit choice whatever the system says', () => {
    expect(resolveTheme('light', true)).toBe('light')
    expect(resolveTheme('dark', false)).toBe('dark')
  })

  it('follows the system when asked to', () => {
    expect(resolveTheme('system', true)).toBe('dark')
    expect(resolveTheme('system', false)).toBe('light')
  })
})

describe('theme preference storage', () => {
  const store = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => store.set(key, value),
  })
  afterEach(() => store.clear())

  it('defaults to dark', () => {
    expect(loadThemePreference()).toBe('dark')
  })

  it('reads back what was saved', () => {
    saveThemePreference('system')
    expect(loadThemePreference()).toBe('system')
  })

  it('ignores unknown values', () => {
    store.set('dbforge.theme', 'sepia')
    expect(loadThemePreference()).toBe('dark')
  })
})
