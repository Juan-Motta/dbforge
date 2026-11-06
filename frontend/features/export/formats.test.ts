import { describe, expect, it } from 'vitest'
import { defaultFileName } from './formats'

describe('defaultFileName', () => {
  it('uses the format extension', () => {
    expect(defaultFileName('orders', 'markdown')).toBe('orders.md')
  })

  it('replaces characters file systems reject', () => {
    expect(defaultFileName('a/b:c', 'csv')).toBe('a_b_c.csv')
  })

  it('falls back to a generic name', () => {
    expect(defaultFileName('  ', 'json')).toBe('export.json')
  })
})
