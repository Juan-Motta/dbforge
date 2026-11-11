import { describe, expect, it } from 'vitest'
import { NO_SSH_TUNNEL, type Connection, type SslMode } from '../../domain/connection'
import { parseConnectionUrl } from './form'
import { connectionUrl } from './url'

const base: Connection = {
  id: 'x',
  name: 'x',
  engine: 'postgres',
  environment: null,
  host: 'db.internal',
  port: 5432,
  database: 'shop',
  username: 'app',
  ssl: { mode: 'prefer', caFile: '', certFile: '', keyFile: '' },
  ssh: NO_SSH_TUNNEL,
  srv: false,
  agentAccess: 'none',
}

const ssl = (mode: SslMode) => ({ ...base.ssl, mode })

describe('connectionUrl', () => {
  it('writes each engine’s URL without a password', () => {
    expect(connectionUrl(base)).toBe('postgresql://app@db.internal:5432/shop')
    expect(connectionUrl({ ...base, engine: 'mariadb', port: 3306 })).toBe(
      'mariadb://app@db.internal:3306/shop',
    )
    expect(connectionUrl({ ...base, engine: 'mysql', username: 'a b', database: '' })).toBe(
      'mysql://a%20b@db.internal:5432',
    )
    expect(connectionUrl({ ...base, engine: 'sqlserver', port: 1433 })).toBe(
      'sqlserver://db.internal:1433;database=shop;user=app',
    )
    expect(connectionUrl({ ...base, engine: 'sqlite', database: '/data/app.db' })).toBe('/data/app.db')
    expect(
      connectionUrl({ ...base, engine: 'cosmosdb', host: 'https://acct.documents.azure.com:443/' }),
    ).toBe('AccountEndpoint=https://acct.documents.azure.com:443/;')
  })

  it('writes Redis and MongoDB URLs the connection dialog reads back', () => {
    const redis = {
      ...base,
      engine: 'redis' as const,
      port: 6380,
      database: '2',
      ssl: ssl('require'),
    }
    expect(connectionUrl(redis)).toBe('rediss://app@db.internal:6380/2#insecure')
    expect(parseConnectionUrl(connectionUrl(redis))?.changes).toMatchObject({
      host: 'db.internal',
      port: '6380',
      username: 'app',
      database: '2',
      ssl: ssl('require'),
    })
    const mongo = { ...base, engine: 'mongodb' as const, port: 27017, ssl: ssl('verify-full') }
    expect(connectionUrl(mongo)).toBe('mongodb://app@db.internal:27017/shop?tls=true')
    expect(parseConnectionUrl(connectionUrl(mongo))?.changes).toMatchObject({
      host: 'db.internal',
      ssl: ssl('verify-full'),
    })
    expect(connectionUrl({ ...mongo, ssl: ssl('require') })).toBe(
      'mongodb://app@db.internal:27017/shop?tls=true&tlsAllowInvalidCertificates=true',
    )
    expect(connectionUrl({ ...mongo, ssl: ssl('disable') })).toBe('mongodb://app@db.internal:27017/shop')
    expect(connectionUrl({ ...mongo, srv: true, host: 'cluster.example.net' })).toBe(
      'mongodb+srv://app@cluster.example.net/shop',
    )
  })

  it('names the SSL mode unless it is the drivers’ default, prefer', () => {
    expect(connectionUrl({ ...base, ssl: ssl('verify-full') })).toBe(
      'postgresql://app@db.internal:5432/shop?sslmode=verify-full',
    )
    expect(connectionUrl({ ...base, ssl: ssl('disable') })).toBe(
      'postgresql://app@db.internal:5432/shop?sslmode=disable',
    )
    const mysql = { ...base, engine: 'mysql' as const, port: 3306 }
    const modes: [SslMode, string][] = [
      ['disable', 'DISABLED'],
      ['require', 'REQUIRED'],
      ['verify-ca', 'VERIFY_CA'],
      ['verify-full', 'VERIFY_IDENTITY'],
    ]
    for (const [mode, name] of modes) {
      expect(connectionUrl({ ...mysql, ssl: ssl(mode) })).toBe(
        `mysql://app@db.internal:3306/shop?ssl-mode=${name}`,
      )
    }
    expect(connectionUrl({ ...mysql, engine: 'mariadb', ssl: ssl('require') })).toBe(
      'mariadb://app@db.internal:3306/shop?ssl-mode=REQUIRED',
    )
  })

  it('never includes certificate files or the SSH tunnel', () => {
    const files = { mode: 'verify-ca' as const, caFile: '/ca.pem', certFile: '/c.pem', keyFile: '/c.key' }
    const tunnel = { ...NO_SSH_TUNNEL, enabled: true, host: 'bastion', username: 'ops' }
    expect(connectionUrl({ ...base, ssl: files, ssh: tunnel })).toBe(
      'postgresql://app@db.internal:5432/shop?sslmode=verify-ca',
    )
  })

  it('takes the password out of a URL saved whole as the host', () => {
    expect(connectionUrl({ ...base, engine: 'mongodb', host: 'mongodb://app:s3cret@h:27017/shop' })).toBe(
      'mongodb://app@h:27017/shop',
    )
    expect(connectionUrl({ ...base, engine: 'redis', host: 'rediss://:s3cret@h:6380' })).toBe(
      'rediss://h:6380',
    )
  })
})
