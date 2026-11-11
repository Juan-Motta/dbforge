import { describe, expect, it } from 'vitest'
import { formatOfPath, mapByName, repeatedColumns } from './mapping'

describe('formatOfPath', () => {
  it('reads the format from the extension', () => {
    expect(formatOfPath('/a/rows.TSV')).toBe('tsv')
    expect(formatOfPath('/a/rows.jsonl')).toBe('json')
    expect(formatOfPath('/a/rows.json')).toBe('json')
    expect(formatOfPath('/a/rows.txt')).toBe('csv')
    expect(formatOfPath('/a/rows')).toBe('csv')
  })
})

describe('mapByName', () => {
  it('sends each field to the column of its name, exact first, then ignoring case', () => {
    expect(mapByName(['id', 'NAME', 'extra'], ['id', 'name', 'email'])).toEqual(['id', 'name', null])
    expect(mapByName(['Name', 'name'], ['name'])).toEqual([null, 'name'])
  })
})

describe('repeatedColumns', () => {
  it('names the columns two fields go into', () => {
    expect(repeatedColumns(['a', null, 'b', 'a', null])).toEqual(['a'])
    expect(repeatedColumns(['a', 'b'])).toEqual([])
  })
})
