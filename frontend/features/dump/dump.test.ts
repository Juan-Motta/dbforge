import { describe, expect, it } from 'vitest'
import { targetNote } from './dump'
import { messages } from './messages'

const en = messages.en

describe('targetNote', () => {
  it('says whether the database is restored into or created first', () => {
    expect(targetNote('shop', ['postgres', 'shop'], en)).toBe('Restores into this existing database.')
    expect(targetNote(' shop_copy ', ['postgres', 'shop'], en)).toBe(
      'Creates this database, then restores into it.',
    )
    expect(targetNote('  ', ['shop'], en)).toBe('Name the database to restore into.')
    expect(targetNote('shop', null, en)).toBe('Restores into this database, creating it if it doesn’t exist.')
  })

  it('speaks the interface language', () => {
    expect(targetNote('shop', ['shop'], messages.es)).toBe('Restaura en esta base de datos existente.')
  })
})
