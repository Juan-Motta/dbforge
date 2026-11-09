import { describe, expect, it } from 'vitest'
import { targetNote } from './dump'

describe('targetNote', () => {
  it('says whether the database is restored into or created first', () => {
    expect(targetNote('shop', ['postgres', 'shop'])).toBe('Restores into this existing database.')
    expect(targetNote(' shop_copy ', ['postgres', 'shop'])).toBe(
      'Creates this database, then restores into it.',
    )
    expect(targetNote('  ', ['shop'])).toBe('Name the database to restore into.')
    expect(targetNote('shop', null)).toBe('Restores into this database, creating it if it doesn’t exist.')
  })
})
