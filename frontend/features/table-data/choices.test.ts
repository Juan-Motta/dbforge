import { describe, expect, it } from 'vitest'
import { choicesOf } from './choices'

describe('choicesOf', () => {
  it('offers an enum’s values in order', () => {
    expect(choicesOf('dbforge.state', ['new', "it's done"])).toEqual([
      { value: 'new', label: 'new' },
      { value: "it's done", label: "it's done" },
    ])
    expect(choicesOf("enum('a','b')", [])).toBeNull()
  })

  it('offers true and false for booleans, as each server writes them', () => {
    expect(choicesOf('boolean')?.map((c) => c.value)).toEqual(['true', 'false'])
    expect(choicesOf('bool')?.map((c) => c.value)).toEqual(['true', 'false'])
    expect(choicesOf('bit')).toEqual([
      { value: '1', label: 'true' },
      { value: '0', label: 'false' },
    ])
    expect(choicesOf('tinyint(1)')?.map((c) => c.value)).toEqual(['1', '0'])
    expect(choicesOf('tinyint(4)')).toBeNull()
    expect(choicesOf('text')).toBeNull()
  })
})
