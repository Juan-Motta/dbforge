import { describe, expect, it } from 'vitest'
import {
  canSubmit,
  emptyForm,
  formFromConnection,
  isValidPort,
  parseConnectionUrl,
  toInput,
  withEngine,
} from './form'

describe('withEngine', () => {
  it('moves an untouched default port to the new engine default', () => {
    expect(withEngine(emptyForm(), 'mysql').port).toBe('3306')
  })

  it('keeps a port the user changed', () => {
    expect(withEngine({ ...emptyForm(), port: '6543' }, 'mysql').port).toBe('6543')
  })
})

describe('isValidPort', () => {
  it.each(['1', '5432', '65535'])('accepts %s', (port) => expect(isValidPort(port)).toBe(true))
  it.each(['', '0', '65536', '54.3', 'abc'])('rejects %j', (port) => expect(isValidPort(port)).toBe(false))
})

describe('canSubmit', () => {
  it('needs a name and a valid port', () => {
    expect(canSubmit(emptyForm())).toBe(false)
    expect(canSubmit({ ...emptyForm(), name: 'local' })).toBe(true)
    expect(canSubmit({ ...emptyForm(), name: 'local', port: 'x' })).toBe(false)
  })

  it('needs a file, not a port, for SQLite', () => {
    const sqlite = { ...withEngine(emptyForm(), 'sqlite'), name: 'local' }
    expect(canSubmit(sqlite)).toBe(false)
    expect(canSubmit({ ...sqlite, database: '/data/app.db' })).toBe(true)
  })
})

describe('toInput', () => {
  it('converts the port and drops an empty password', () => {
    const input = toInput({ ...emptyForm(), name: 'local' }, 'abc')
    expect(input).toMatchObject({ id: 'abc', port: 5432 })
    expect(input).not.toHaveProperty('password', '')
    expect(input.password).toBeUndefined()
  })

  it('sends no server details for a SQLite file', () => {
    const form = {
      ...emptyForm(),
      engine: 'sqlite' as const,
      name: 'local',
      database: '/data/app.db',
      password: 'x',
    }
    expect(toInput(form)).toMatchObject({ host: '', port: 0, username: '', database: '/data/app.db' })
    expect(toInput(form).password).toBeUndefined()
  })

  it('keeps a typed password', () => {
    expect(toInput({ ...emptyForm(), password: 's3cret' }).password).toBe('s3cret')
  })

  it('round-trips a saved connection without its password', () => {
    const saved = {
      id: 'abc',
      name: 'prod',
      engine: 'postgres' as const,
      environment: 'production' as const,
      host: 'db',
      port: 5433,
      database: 'shop',
      username: 'app',
      tls: false,
      allowSelfSigned: false,
      srv: false,
    }
    expect(toInput(formFromConnection(saved), saved.id)).toEqual({ ...saved, password: undefined })
  })
})

describe('parseConnectionUrl', () => {
  it('splits a Redis URL into fields, TLS included', () => {
    expect(parseConnectionUrl('rediss://app:s%40cret@cache.example.com:6380/2#insecure')).toEqual({
      changes: {
        engine: 'redis',
        host: 'cache.example.com',
        port: '6380',
        username: 'app',
        password: 's@cret',
        database: '2',
        tls: true,
        allowSelfSigned: true,
        srv: false,
      },
      ignored: [],
    })
    const plain = parseConnectionUrl('redis://localhost')!.changes
    expect([plain.port, plain.tls, plain.database, plain.username]).toEqual(['6379', false, '', ''])
  })

  it('reads MongoDB seed lists and TLS options, and lists what it leaves out', () => {
    const atlas = parseConnectionUrl(
      'mongodb+srv://ana:pw@cluster0.ab12c.mongodb.net/shop?retryWrites=true&w=majority&authSource=admin',
    )!
    expect(atlas.changes).toMatchObject({
      engine: 'mongodb',
      host: 'cluster0.ab12c.mongodb.net',
      database: 'shop',
      srv: true,
      tls: true,
      allowSelfSigned: false,
    })
    expect(atlas.ignored).toEqual(['retryWrites=true', 'w=majority'])

    const replica = parseConnectionUrl(
      'mongodb://a:27017,b:27018/?tls=true&tlsAllowInvalidCertificates=true',
    )!
    expect(replica.changes).toMatchObject({ host: 'a', port: '27017', tls: true, allowSelfSigned: true })
    expect(replica.ignored).toEqual(['hosts b:27018'])
  })

  it('leaves anything else alone', () => {
    expect(parseConnectionUrl('localhost')).toBeNull()
    expect(parseConnectionUrl('postgres://x')).toBeNull()
  })
})

describe('TLS options', () => {
  it('opens a connection saved with a URL host as fields', () => {
    const saved = {
      id: '1',
      name: 'cache',
      engine: 'redis' as const,
      environment: null,
      host: 'rediss://127.0.0.1:56380/#insecure',
      port: 0,
      database: '',
      username: '',
      tls: false,
      allowSelfSigned: false,
      srv: false,
    }
    expect(formFromConnection(saved)).toMatchObject({
      host: '127.0.0.1',
      port: '56380',
      tls: true,
      allowSelfSigned: true,
    })
  })

  it('sends the switches, and never a self-signed exception without TLS', () => {
    const redis = { ...withEngine(emptyForm(), 'redis'), name: 'r', tls: false, allowSelfSigned: true }
    expect(toInput(redis)).toMatchObject({ tls: false, allowSelfSigned: false, srv: false })
    expect(toInput({ ...redis, tls: true })).toMatchObject({ tls: true, allowSelfSigned: true })
  })

  it('sends no database for Redis: the explorer opens any of them', () => {
    const redis = { ...withEngine(emptyForm(), 'redis'), name: 'r', database: '3' }
    expect(toInput(redis).database).toBe('')
  })

  it('needs no port for a MongoDB seed list', () => {
    const atlas = { ...withEngine(emptyForm(), 'mongodb'), name: 'atlas', srv: true, port: '' }
    expect(canSubmit(atlas)).toBe(true)
  })
})
