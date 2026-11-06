import { describe, expect, it } from 'vitest'
import { connectionAddress, defaultSchema, sqlDialectOf, type Connection } from './connection'
import { tableRefFromPath } from './table'

describe('tableRefFromPath', () => {
  it('reads database, schema and table', () => {
    expect(tableRefFromPath(['shop', 'public', 'orders'])).toEqual({
      database: 'shop',
      schema: 'public',
      name: 'orders',
    })
  })

  it('uses main for a SQLite file', () => {
    expect(tableRefFromPath(['orders'])).toEqual({ database: 'main', schema: 'main', name: 'orders' })
  })

  it('uses the database as the schema when there is none (MySQL)', () => {
    expect(tableRefFromPath(['shop', 'orders'])).toEqual({ database: 'shop', schema: 'shop', name: 'orders' })
  })
})

describe('engines', () => {
  it('picks the default schema', () => {
    expect(defaultSchema('postgres', 'shop')).toBe('public')
    expect(defaultSchema('mysql', 'shop')).toBe('shop')
    expect(defaultSchema('sqlite', '/tmp/app.db')).toBe('main')
    expect(defaultSchema('sqlserver', 'shop')).toBe('dbo')
  })

  it('picks the SQL dialect', () => {
    expect(sqlDialectOf('mysql')).toBe('mysql')
    expect(sqlDialectOf('postgres')).toBe('postgres')
    expect(sqlDialectOf('sqlite')).toBe('sqlite')
    expect(sqlDialectOf('sqlserver')).toBe('sqlserver')
    expect(sqlDialectOf(undefined)).toBe('postgres')
  })
})

describe('connectionAddress', () => {
  const base: Connection = {
    id: '1',
    name: 'x',
    engine: 'postgres',
    environment: null,
    host: 'db.local',
    port: 5432,
    database: 'shop',
    username: 'app',
  }

  it('shows user, host and port for servers', () => {
    expect(connectionAddress(base)).toBe('app@db.local:5432')
  })

  it('shows the file for SQLite', () => {
    expect(connectionAddress({ ...base, engine: 'sqlite', database: '/data/app.db' })).toBe('/data/app.db')
  })
})
