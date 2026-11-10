import { describe, expect, it } from 'vitest'
import type { Connection } from '../../domain/connection'
import { connectionFacts } from './connectionFacts'
import { messages as shared } from '../../lib/messages'
import { messages } from './messages'

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
  agentAccess: 'none',
}

describe('connectionFacts', () => {
  it('lists the server, address and database the status bar used to show', () => {
    expect(connectionFacts(base, 'PostgreSQL 16.2', messages.en, shared.en)).toEqual([
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
    expect(connectionFacts(redis, null, messages.en, shared.en)).toEqual([
      ['Server', 'Checking…'],
      ['Address', 'default user@db.internal:5432'],
      ['TLS', 'On · self-signed certificate accepted'],
    ])
    expect(
      connectionFacts({ ...base, engine: 'mongodb', database: '' }, 'x', messages.en, shared.en),
    ).toContainEqual(['TLS', 'Off'])
  })

  it('shows a SQLite file as its path', () => {
    const sqlite = { ...base, engine: 'sqlite' as const, host: '', port: 0, database: '/data/app.db' }
    expect(connectionFacts(sqlite, 'SQLite 3.46', messages.en, shared.en)).toEqual([
      ['Server', 'SQLite 3.46'],
      ['File', '/data/app.db'],
    ])
  })

  it('writes the labels in Spanish', () => {
    expect(
      connectionFacts({ ...base, engine: 'mongodb', database: '' }, null, messages.es, shared.es),
    ).toContainEqual(['TLS', 'Desactivado'])
    expect(connectionFacts(base, null, messages.es, shared.es)[0]).toEqual(['Servidor', 'Comprobando…'])
    const noUser = { ...base, username: '' }
    expect(connectionFacts(noUser, null, messages.es, shared.es)[1]).toEqual([
      'Dirección',
      'usuario por defecto@db.internal:5432',
    ])
  })
})
