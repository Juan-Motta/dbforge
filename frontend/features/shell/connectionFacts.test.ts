import { describe, expect, it } from 'vitest'
import { NO_SSH_TUNNEL, type Connection } from '../../domain/connection'
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
  ssl: { mode: 'prefer', caFile: '', certFile: '', keyFile: '' },
  ssh: NO_SSH_TUNNEL,
  srv: false,
  agentAccess: 'none',
}

describe('connectionFacts', () => {
  it('lists the server, address and database the status bar used to show', () => {
    expect(connectionFacts(base, 'PostgreSQL 16.2', messages.en, shared.en)).toEqual([
      ['Server', 'PostgreSQL 16.2'],
      ['Address', 'app@db.internal:5432'],
      ['Database', 'shop'],
      ['SSL', 'Prefer (when the server offers it)'],
    ])
  })

  it('adds the SSH tunnel when it is on', () => {
    const ssh = { ...NO_SSH_TUNNEL, enabled: true, host: 'bastion', port: 2222, username: 'ops' }
    expect(connectionFacts({ ...base, ssh }, 'x', messages.en, shared.en)).toContainEqual([
      'SSH',
      'ops@bastion:2222',
    ])
    expect(
      connectionFacts({ ...base, ssh: { ...ssh, enabled: false } }, 'x', messages.en, shared.en),
    ).not.toContainEqual(['SSH', 'ops@bastion:2222'])
  })

  it('adds the SSL mode where the connection can use it, and says when the server is still being asked', () => {
    const redis = {
      ...base,
      engine: 'redis' as const,
      database: '',
      username: '',
      ssl: { ...base.ssl, mode: 'require' as const },
    }
    expect(connectionFacts(redis, null, messages.en, shared.en)).toEqual([
      ['Server', 'Checking…'],
      ['Address', 'default user@db.internal:5432'],
      ['SSL', 'Require (any certificate)'],
    ])
    const mongo = {
      ...base,
      engine: 'mongodb' as const,
      database: '',
      ssl: { ...base.ssl, mode: 'disable' as const },
    }
    expect(connectionFacts(mongo, 'x', messages.en, shared.en)).toContainEqual(['SSL', 'Off'])
  })

  it('shows a SQLite file as its path', () => {
    const sqlite = { ...base, engine: 'sqlite' as const, host: '', port: 0, database: '/data/app.db' }
    expect(connectionFacts(sqlite, 'SQLite 3.46', messages.en, shared.en)).toEqual([
      ['Server', 'SQLite 3.46'],
      ['File', '/data/app.db'],
    ])
  })

  it('writes the labels in Spanish', () => {
    const mongo = {
      ...base,
      engine: 'mongodb' as const,
      database: '',
      ssl: { ...base.ssl, mode: 'disable' as const },
    }
    expect(connectionFacts(mongo, null, messages.es, shared.es)).toContainEqual(['SSL', 'Desactivado'])
    expect(connectionFacts(base, null, messages.es, shared.es)[0]).toEqual(['Servidor', 'Comprobando…'])
    const noUser = { ...base, username: '' }
    expect(connectionFacts(noUser, null, messages.es, shared.es)[1]).toEqual([
      'Dirección',
      'usuario por defecto@db.internal:5432',
    ])
  })
})
