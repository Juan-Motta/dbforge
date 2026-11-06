import { describe, expect, it } from 'vitest'
import type { TableStructure } from './api'
import { columnKeys, formatBytes, summary } from './format'

const structure = (overrides: Partial<TableStructure> = {}): TableStructure => ({
  kind: 'table',
  rowEstimate: 48213,
  dataBytes: 9.4 * 1024 * 1024,
  indexBytes: 3.1 * 1024 * 1024,
  comment: null,
  columns: [],
  indexes: [],
  foreignKeys: [],
  triggers: [],
  ddl: '',
  ...overrides,
})

const index = (
  name: string,
  columns: string[],
  flags: { primary?: boolean; unique?: boolean; condition?: string } = {},
) => ({
  name,
  method: 'btree',
  columns,
  condition: flags.condition ?? null,
  bytes: 0,
  primary: flags.primary ?? false,
  unique: flags.unique ?? flags.primary ?? false,
})

describe('columnKeys', () => {
  it('marks primary, foreign, unique and indexed columns, strongest first', () => {
    const keys = columnKeys(
      structure({
        indexes: [
          index('pk', ['id'], { primary: true }),
          index('uq_email', ['email'], { unique: true }),
          index('ix_country_segment', ['country', 'segment']),
          index('ix_referrer', ['referrer_id']),
        ],
        foreignKeys: [
          {
            name: 'fk',
            columns: ['referrer_id'],
            referencedTable: 'customers',
            referencedColumns: ['id'],
            onUpdate: 'NO ACTION',
            onDelete: 'SET NULL',
          },
        ],
      }),
    )

    expect(Object.fromEntries(keys)).toEqual({
      id: ['PK'],
      email: ['UNI'],
      country: ['IDX'],
      segment: ['IDX'],
      referrer_id: ['FK', 'IDX'],
    })
  })

  it('treats a multi-column unique index as a plain index', () => {
    const keys = columnKeys(structure({ indexes: [index('uq', ['a', 'b'], { unique: true })] }))
    expect(keys.get('a')).toEqual(['IDX'])
  })

  it('treats a partial unique index as a plain index', () => {
    const partial = index('uq_open', ['kb_id'], { unique: true, condition: "status = 'in_progress'" })
    expect(columnKeys(structure({ indexes: [partial] })).get('kb_id')).toEqual(['IDX'])
  })

  it('unquotes mixed-case names and ignores expressions', () => {
    const keys = columnKeys(structure({ indexes: [index('a', ['"Note"']), index('b', ['lower(email)'])] }))
    expect(keys.get('Note')).toEqual(['IDX'])
    expect(keys.has('email')).toBe(false)
  })
})

describe('formatBytes', () => {
  it.each([
    [0, '0 B'],
    [512, '512 B'],
    [16 * 1024, '16 kB'],
    [1.5 * 1024, '1.5 kB'],
    [9.44 * 1024 * 1024, '9.4 MB'],
    [3 * 1024 ** 4, '3.0 TB'],
  ])('%d bytes → %s', (bytes, text) => {
    expect(formatBytes(bytes)).toBe(text)
  })
})

describe('summary', () => {
  it('shows rows and sizes for tables', () => {
    expect(summary(structure())).toBe('~48,213 rows · 9.4 MB data · 3.1 MB indexes')
  })

  it('leaves out what a view or a never-analyzed table lacks', () => {
    expect(summary(structure({ kind: 'view', rowEstimate: null }))).toBe('')
  })
})
