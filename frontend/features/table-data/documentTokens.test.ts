import { describe, expect, it } from 'vitest'
import { documentTokens } from './documentTokens'

describe('documentTokens', () => {
  it('tells keys, strings, numbers, literals and mongosh calls apart', () => {
    const text =
      '{\n  "_id": ObjectId("65a0"),\n  "n": NumberLong(5),\n  "ok": true,\n  "note": "a: \\"b\\"",\n  "x": -1.5e3\n}'
    const tokens = documentTokens(text)
    expect(tokens.map((t) => t.text).join('')).toBe(text)
    const kinds = tokens.filter((t) => t.kind !== 'plain').map((t) => [t.kind, t.text])
    expect(kinds).toEqual([
      ['key', '"_id"'],
      ['call', 'ObjectId'],
      ['string', '"65a0"'],
      ['key', '"n"'],
      ['call', 'NumberLong'],
      ['number', '5'],
      ['key', '"ok"'],
      ['literal', 'true'],
      ['key', '"note"'],
      ['string', '"a: \\"b\\""'],
      ['key', '"x"'],
      ['number', '-1.5e3'],
    ])
  })

  it('keeps numbers inside words and strings whole', () => {
    const kinds = documentTokens('{"k2": "v1", "ok": null}').map((t) => t.kind)
    expect(kinds.filter((k) => k === 'number')).toEqual([])
  })
})
