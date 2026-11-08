import { describe, expect, it } from 'vitest'
import { compactJson, isJsonDocument, jsonError, prettyJson } from './cellText'

describe('prettyJson', () => {
  it('indents objects and arrays by two spaces', () => {
    expect(prettyJson('{"a":1,"b":[true,null]}')).toBe('{\n  "a": 1,\n  "b": [\n    true,\n    null\n  ]\n}')
  })

  it('keeps big integers and decimals exactly as written', () => {
    expect(prettyJson('{"id":12345678901234567890,"price":1.50}')).toBe(
      '{\n  "id": 12345678901234567890,\n  "price": 1.50\n}',
    )
  })

  it('leaves punctuation and escaped quotes inside strings alone', () => {
    expect(prettyJson('{"say":"a, b: {c} \\"d\\""}')).toBe('{\n  "say": "a, b: {c} \\"d\\""\n}')
  })

  it('keeps empty objects and arrays on one line', () => {
    expect(prettyJson('{"a":{},"b":[]}')).toBe('{\n  "a": {},\n  "b": []\n}')
  })
})

describe('compactJson', () => {
  it('removes whitespace outside strings only', () => {
    expect(compactJson('{\n  "a b": 1,\n  "c": [ 2 ]\n}')).toBe('{"a b":1,"c":[2]}')
  })

  it('undoes prettyJson', () => {
    const text = '{"id":12345678901234567890,"tags":["x, y"],"o":{}}'
    expect(compactJson(prettyJson(text))).toBe(text)
  })
})

describe('isJsonDocument', () => {
  it('accepts objects and arrays only', () => {
    expect(isJsonDocument(' {"a": 1} ')).toBe(true)
    expect(isJsonDocument('[1]')).toBe(true)
    expect(isJsonDocument('42')).toBe(false)
    expect(isJsonDocument('"text"')).toBe(false)
    expect(isJsonDocument('{oops}')).toBe(false)
  })
})

describe('jsonError', () => {
  it('says why text is not JSON', () => {
    expect(jsonError('{"a":1}')).toBeNull()
    expect(jsonError('{"a":}')).toEqual(expect.any(String))
  })
})
