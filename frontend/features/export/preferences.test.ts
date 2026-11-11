import { describe, expect, it } from 'vitest'
import { DEFAULT_OPTIONS, exportOptions } from './formats'
import { readChoice } from './preferences'

describe('readChoice', () => {
  it('brings back the last format and options', () => {
    const kept = {
      format: 'sql',
      options: { header: false, nullText: 'NULL', jsonStyle: 'lines', rowsPerInsert: 500 },
    }
    expect(readChoice(JSON.stringify(kept))).toEqual(kept)
  })

  it('falls back to CSV and the defaults when nothing is kept or it is not understood', () => {
    const fallback = { format: 'csv', options: DEFAULT_OPTIONS }
    expect(readChoice(null)).toEqual(fallback)
    expect(readChoice('not json')).toEqual(fallback)
    expect(
      readChoice(JSON.stringify({ format: 'xlsx', options: { rowsPerInsert: 7, header: 'yes' } })),
    ).toEqual(fallback)
  })
})

describe('exportOptions', () => {
  const all = ['id', 'name', 'email']

  it('sends no column list when every column is chosen', () => {
    expect(exportOptions(DEFAULT_OPTIONS, ['email', 'id', 'name'], all).columns).toBeUndefined()
  })

  it('sends the chosen columns in the table order', () => {
    expect(exportOptions(DEFAULT_OPTIONS, ['email', 'id'], all).columns).toEqual(['id', 'email'])
  })
})
