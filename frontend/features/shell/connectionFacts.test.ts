import { describe, expect, it } from 'vitest'
import type { Connection } from '../../domain/connection'
import { connectionFacts } from './connectionFacts'

const base: Connection = {
  id: '1',
  name: 'prod',
  engine: 'postgres',
  environment: 'production',
  host: 'db.internal',
  port: 5432,
  database: 'shop',
  username: 'app',
  tls: false,
  allowSelfSigned: false,
  srv: false,
}

describe('connectionFacts', () => {
  it('lists the server, address and database the status bar used to show', () => {
    expect(connectionFacts(base, 'PostgreSQL 16.2')).toEqual([
      ['Server', 'PostgreSQL 16.2'],
      ['Address', 'app@db.internal:5432'],
      ['Database', 'shop'],
    ])
  })

  it('adds TLS where the connection can use it, and says when the server is still being asked', () => {
    const redis = {
      ...base,
      engine: 'redis' as const,
      database: '',
      username: '',
      tls: true,
      allowSelfSigned: true,
    }
    expect(connectionFacts(redis, null)).toEqual([
      ['Server', 'Checking…'],
      ['Address', 'default user@db.internal:5432'],
      ['TLS', 'On · self-signed certificate accepted'],
    ])
    expect(connectionFacts({ ...base, engine: 'mongodb', database: '' }, 'x')).toContainEqual(['TLS', 'Off'])
  })

  it('shows a SQLite file as its path', () => {
    const sqlite = { ...base, engine: 'sqlite' as const, host: '', port: 0, database: '/data/app.db' }
    expect(connectionFacts(sqlite, 'SQLite 3.46')).toEqual([
      ['Server', 'SQLite 3.46'],
      ['File', '/data/app.db'],
    ])
  })
})
