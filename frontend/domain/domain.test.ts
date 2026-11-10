import { describe, expect, it } from 'vitest'
import {
  connectionAddress,
  defaultSchema,
  ENGINE_LIST,
  ENGINES,
  hasDiagram,
  isRelationalDialect,
  sqlDialectOf,
  type Connection,
} from './connection'
import { tableRefFromPath } from './table'
import { messages } from '../lib/messages'

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
    // A MongoDB database holds its collections directly.
    expect(defaultSchema('mongodb', 'shop')).toBe('shop')
    expect(defaultSchema('cosmosdb', 'shop')).toBe('shop')
  })

  it('picks the SQL dialect', () => {
    expect(sqlDialectOf('mysql')).toBe('mysql')
    expect(sqlDialectOf('postgres')).toBe('postgres')
    expect(sqlDialectOf('sqlite')).toBe('sqlite')
    expect(sqlDialectOf('sqlserver')).toBe('sqlserver')
    expect(sqlDialectOf('mongodb')).toBe('mongodb')
    expect(sqlDialectOf('cosmosdb')).toBe('cosmosdb')
    expect(sqlDialectOf(undefined)).toBe('postgres')
  })
})

describe('hasDiagram', () => {
  it('is false for engines that declare no relationships', () => {
    expect(hasDiagram('postgres')).toBe(true)
    expect(hasDiagram('sqlserver')).toBe(true)
    expect(hasDiagram('mongodb')).toBe(false)
    expect(hasDiagram('cosmosdb')).toBe(false)
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
    tls: false,
    allowSelfSigned: false,
    srv: false,
    agentAccess: 'none',
  }

  it('shows user, host and port for servers', () => {
    expect(connectionAddress(base)).toBe('app@db.local:5432')
  })

  it('shows only host and port for an account key', () => {
    const cosmos = { ...base, engine: 'cosmosdb' as const, host: 'shop.documents.azure.com', port: 443 }
    expect(connectionAddress({ ...cosmos, username: '' })).toBe('shop.documents.azure.com:443')
  })

  it('names the default user in the interface language when there is none', () => {
    expect(connectionAddress({ ...base, username: '' })).toBe('default user@db.local:5432')
    expect(connectionAddress({ ...base, username: '' }, messages.es)).toBe(
      'usuario por defecto@db.local:5432',
    )
  })

  it('shows the file for SQLite', () => {
    expect(connectionAddress({ ...base, engine: 'sqlite', database: '/data/app.db' })).toBe('/data/app.db')
  })
})

describe('ENGINES', () => {
  it('lists SQL engines first, then the others, as the connection form groups them', () => {
    expect(ENGINE_LIST).toEqual([
      'postgres',
      'mysql',
      'mariadb',
      'sqlite',
      'sqlserver',
      'mongodb',
      'cosmosdb',
      'redis',
    ])
    const relational = ENGINE_LIST.filter((engine) => ENGINES[engine].family === 'relational')
    expect(ENGINE_LIST.slice(0, relational.length)).toEqual(relational)
  })

  it('tells SQL dialects from document ones', () => {
    expect(['postgres', 'mysql', 'sqlite', 'sqlserver'].every((d) => isRelationalDialect(d as never))).toBe(
      true,
    )
    expect(isRelationalDialect('mongodb') || isRelationalDialect('cosmosdb')).toBe(false)
  })
})
