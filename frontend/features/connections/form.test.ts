import { describe, expect, it } from 'vitest'
import { NO_SSH_TUNNEL, type Connection } from '../../domain/connection'
import { messages } from './messages'
import {
  canSubmit,
  canTest,
  emptyForm,
  formFromConnection,
  isValidPort,
  parseConnectionUrl,
  toInput,
  withEngine,
  type ConnectionForm,
} from './form'

const ssl = (mode: Connection['ssl']['mode']) => ({ mode, caFile: '', certFile: '', keyFile: '' })

/** A form with the tunnel on and complete, for `engine`. */
function tunneled(engine: Connection['engine'] = 'postgres'): ConnectionForm {
  return {
    ...withEngine({ ...emptyForm(), name: 'n' }, engine),
    ssh: { ...NO_SSH_TUNNEL, port: '22', enabled: true, host: 'bastion', username: 'ops' },
  }
}

describe('withEngine', () => {
  it('moves an untouched default port to the new engine default', () => {
    expect(withEngine(emptyForm(), 'mysql').port).toBe('3306')
  })

  it('keeps a port the user changed', () => {
    expect(withEngine({ ...emptyForm(), port: '6543' }, 'mysql').port).toBe('6543')
  })

  it('empties the host for an account URI, and back', () => {
    const cosmos = withEngine(emptyForm(), 'cosmosdb')
    expect(cosmos).toMatchObject({ host: '', port: '443' })
    expect(withEngine({ ...cosmos, host: 'https://a.documents.azure.com/' }, 'mongodb').host).toBe(
      'localhost',
    )
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
      ssl: { mode: 'verify-ca' as const, caFile: '/ca.pem', certFile: '/c.pem', keyFile: '/c.key' },
      ssh: {
        enabled: true,
        host: 'bastion',
        port: 2222,
        username: 'ops',
        auth: 'key' as const,
        keyFile: '/id_ed25519',
      },
      srv: false,
      agentAccess: 'none' as const,
    }
    expect(toInput(formFromConnection(saved), saved.id)).toEqual({
      ...saved,
      password: undefined,
      sshSecret: undefined,
    })
  })

  it('keeps or sends the SSH secret as it does the password', () => {
    expect(toInput(tunneled()).sshSecret).toBeUndefined()
    expect(toInput({ ...tunneled(), sshSecret: 'pass' }).sshSecret).toBe('pass')
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
        ssl: ssl('require'),
        srv: false,
      },
      ignored: [],
    })
    expect(parseConnectionUrl('rediss://cache:6380')!.changes.ssl).toEqual(ssl('verify-full'))
    const plain = parseConnectionUrl('redis://localhost')!.changes
    expect([plain.port, plain.ssl?.mode, plain.database, plain.username]).toEqual(['6379', 'disable', '', ''])
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
      ssl: ssl('verify-full'),
    })
    expect(atlas.ignored).toEqual(['retryWrites=true', 'w=majority'])

    const replica = parseConnectionUrl(
      'mongodb://a:27017,b:27018/?tls=true&tlsAllowInvalidCertificates=true',
    )!
    expect(replica.changes).toMatchObject({ host: 'a', port: '27017', ssl: ssl('require') })
    expect(parseConnectionUrl('mongodb://a/?ssl=true')!.changes.ssl).toEqual(ssl('verify-full'))
    expect(parseConnectionUrl('mongodb://a/?tls=true&tlsInsecure=true')!.changes.ssl).toEqual(ssl('require'))
    expect(parseConnectionUrl('mongodb://a/')!.changes.ssl).toEqual(ssl('disable'))
    expect(replica.ignored).toEqual(['hosts b:27018'])
    expect(parseConnectionUrl('mongodb://a:27017,b:27018/', messages.es)!.ignored).toEqual([
      'hosts adicionales b:27018',
    ])
  })

  it('reads a Cosmos DB connection string, key padding included', () => {
    const azure = parseConnectionUrl(
      'AccountEndpoint=https://shop.documents.azure.com:443/;AccountKey=abc+/def==;Database=x;',
    )!
    expect(azure.changes).toEqual({
      engine: 'cosmosdb',
      host: 'https://shop.documents.azure.com/',
      username: '',
      password: 'abc+/def==',
      srv: false,
    })
    expect(azure.ignored).toEqual(['Database=x'])
    expect(parseConnectionUrl('AccountEndpoint=not a url;AccountKey=k')).toBeNull()
    // A bare URI is what the URI field holds; nothing to fill in.
    expect(parseConnectionUrl('https://shop.documents.azure.com:443/')).toBeNull()
  })

  it('leaves anything else alone', () => {
    expect(parseConnectionUrl('localhost')).toBeNull()
    expect(parseConnectionUrl('postgres://x')).toBeNull()
    expect(parseConnectionUrl('https://')).toBeNull()
  })
})

describe('SSL options', () => {
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
      ssl: ssl('disable'),
      ssh: NO_SSH_TUNNEL,
      srv: false,
      agentAccess: 'none' as const,
    }
    expect(formFromConnection(saved)).toMatchObject({
      host: '127.0.0.1',
      port: '56380',
      ssl: ssl('require'),
    })
  })

  it('starts new connections on prefer where the engine has it, else without SSL', () => {
    expect(emptyForm().ssl.mode).toBe('prefer')
    const mode = (engine: Connection['engine']) => withEngine(emptyForm(), engine).ssl.mode
    expect(['mysql', 'mariadb', 'sqlserver'].map((e) => mode(e as Connection['engine']))).toEqual([
      'prefer',
      'prefer',
      'prefer',
    ])
    expect(['mongodb', 'redis', 'sqlite', 'cosmosdb'].map((e) => mode(e as Connection['engine']))).toEqual([
      'disable',
      'disable',
      'disable',
      'disable',
    ])
  })

  it('keeps the SSL mode when the new engine has it', () => {
    const verified = { ...emptyForm(), ssl: ssl('verify-full') }
    expect(withEngine(verified, 'redis').ssl.mode).toBe('verify-full')
    expect(withEngine({ ...emptyForm(), ssl: ssl('verify-ca') }, 'sqlserver').ssl.mode).toBe('prefer')
  })

  it('sends the mode and files, and no SSL or tunnel for a file', () => {
    const redis = { ...withEngine(emptyForm(), 'redis'), name: 'r', ssl: ssl('require') }
    expect(toInput(redis)).toMatchObject({ ssl: ssl('require'), srv: false })
    const sqlite = { ...tunneled('sqlite'), database: '/a.db', ssl: ssl('require') }
    expect(toInput(sqlite)).toMatchObject({ ssl: ssl('disable'), ssh: NO_SSH_TUNNEL })
  })

  it('always encrypts a MongoDB seed list', () => {
    const atlas = { ...withEngine(emptyForm(), 'mongodb'), name: 'a', srv: true }
    expect(toInput(atlas).ssl.mode).toBe('verify-full')
    expect(toInput({ ...atlas, ssl: ssl('require') }).ssl.mode).toBe('require')
  })

  it('sends no database for Redis: the explorer opens any of them', () => {
    const redis = { ...withEngine(emptyForm(), 'redis'), name: 'r', database: '3' }
    expect(toInput(redis).database).toBe('')
  })

  it('splits an account URI into host, port and TLS, with no user or database', () => {
    const form = { ...withEngine(emptyForm(), 'cosmosdb'), name: 'c', username: 'x', database: 'y' }
    const https = { ...form, host: ' https://shop.documents.azure.com:443/ ' }
    expect(toInput({ ...https, ssl: ssl('require') })).toMatchObject({
      host: 'shop.documents.azure.com',
      port: 443,
      ssl: ssl('require'),
      ssh: NO_SSH_TUNNEL,
      username: '',
      database: '',
    })
    expect(toInput(https).ssl.mode).toBe('verify-full')
    const emulator = toInput({ ...form, host: 'http://127.0.0.1:8081/', ssl: ssl('require') })
    expect(emulator).toMatchObject({ host: '127.0.0.1', port: 8081, ssl: ssl('disable') })
    expect(canTest({ ...form, host: 'shop.documents.azure.com' })).toBe(false)
    expect(canTest({ ...form, host: 'https://shop.documents.azure.com' })).toBe(true)
  })

  it('opens a saved account connection as its URI', () => {
    const saved = {
      id: '1',
      name: 'emulator',
      engine: 'cosmosdb' as const,
      environment: null,
      host: '127.0.0.1',
      port: 8081,
      database: '',
      username: '',
      ssl: ssl('disable'),
      ssh: NO_SSH_TUNNEL,
      srv: false,
      agentAccess: 'none' as const,
    }
    expect(formFromConnection(saved).host).toBe('http://127.0.0.1:8081/')
    const https = formFromConnection({ ...saved, ssl: ssl('require'), port: 443 })
    expect(https).toMatchObject({ host: 'https://127.0.0.1:443/', ssl: ssl('require') })
  })

  it('needs no port for a MongoDB seed list', () => {
    const atlas = { ...withEngine(emptyForm(), 'mongodb'), name: 'atlas', srv: true, port: '' }
    expect(canSubmit(atlas)).toBe(true)
  })
})

describe('SSH tunnel', () => {
  it('needs the SSH host, user, a valid port and, for key auth, the key file', () => {
    expect(canTest(tunneled())).toBe(true)
    expect(canTest({ ...tunneled(), ssh: { ...tunneled().ssh, host: ' ' } })).toBe(false)
    expect(canTest({ ...tunneled(), ssh: { ...tunneled().ssh, username: '' } })).toBe(false)
    expect(canTest({ ...tunneled(), ssh: { ...tunneled().ssh, port: '0' } })).toBe(false)
    expect(canTest({ ...tunneled(), ssh: { ...tunneled().ssh, auth: 'key' } })).toBe(false)
    expect(canTest({ ...tunneled(), ssh: { ...tunneled().ssh, auth: 'key', keyFile: '/k' } })).toBe(true)
    expect(canTest({ ...tunneled(), ssh: { ...tunneled().ssh, auth: 'agent' } })).toBe(true)
    // Off, what it holds doesn't matter.
    expect(canTest({ ...tunneled(), ssh: { ...tunneled().ssh, enabled: false, host: '' } })).toBe(true)
  })

  it('can’t check the server name through it, except on SQL Server', () => {
    expect(canTest({ ...tunneled('postgres'), ssl: ssl('verify-full') })).toBe(false)
    expect(canTest({ ...tunneled('postgres'), ssl: ssl('verify-ca') })).toBe(true)
    expect(canTest({ ...tunneled('sqlserver'), ssl: ssl('verify-full') })).toBe(true)
  })

  it('doesn’t reach a MongoDB seed list', () => {
    expect(canTest({ ...tunneled('mongodb'), srv: true })).toBe(false)
  })

  it('sends the port as a number, and sends an off tunnel as typed', () => {
    expect(toInput(tunneled()).ssh).toEqual({
      ...NO_SSH_TUNNEL,
      enabled: true,
      host: 'bastion',
      username: 'ops',
    })
    const off = { ...tunneled(), ssh: { ...tunneled().ssh, enabled: false } }
    expect(toInput(off).ssh).toMatchObject({ enabled: false, host: 'bastion' })
  })
})

describe('client certificate', () => {
  it('needs the key with the certificate, where they are separate files', () => {
    const pg = { ...emptyForm(), name: 'n', ssl: { ...ssl('require'), certFile: '/c.pem' } }
    expect(canTest(pg)).toBe(false)
    expect(canTest({ ...pg, ssl: { ...pg.ssl, keyFile: '/c.key' } })).toBe(true)
    expect(canTest({ ...withEngine(pg, 'mongodb'), ssl: { ...ssl('require'), certFile: '/c.pem' } })).toBe(
      true,
    )
  })
})
