import { describe, expect, it } from 'vitest'
import { defaultSchema, sqlDialectOf } from './connection'
import { tableRefFromPath } from './table'

describe('tableRefFromPath', () => {
  it('reads database, schema and table', () => {
    expect(tableRefFromPath(['shop', 'public', 'orders'])).toEqual({
      database: 'shop',
      schema: 'public',
      name: 'orders',
    })
  })

  it('uses the database as the schema when there is none (MySQL)', () => {
    expect(tableRefFromPath(['shop', 'orders'])).toEqual({ database: 'shop', schema: 'shop', name: 'orders' })
  })
})

describe('engines', () => {
  it('picks the default schema', () => {
    expect(defaultSchema('postgres', 'shop')).toBe('public')
    expect(defaultSchema('mysql', 'shop')).toBe('shop')
  })

  it('picks the SQL dialect', () => {
    expect(sqlDialectOf('mysql')).toBe('mysql')
    expect(sqlDialectOf('postgres')).toBe('postgres')
    expect(sqlDialectOf(undefined)).toBe('postgres')
  })
})
