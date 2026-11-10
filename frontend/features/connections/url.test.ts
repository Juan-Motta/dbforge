import { describe, expect, it } from 'vitest'
import type { Connection } from '../../domain/connection'
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
  tls: false,
  allowSelfSigned: false,
  srv: false,
}

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
      tls: true,
      allowSelfSigned: true,
    }
    expect(connectionUrl(redis)).toBe('rediss://app@db.internal:6380/2#insecure')
    expect(parseConnectionUrl(connectionUrl(redis))?.changes).toMatchObject({
      host: 'db.internal',
      port: '6380',
      username: 'app',
      database: '2',
      tls: true,
      allowSelfSigned: true,
    })
    const mongo = { ...base, engine: 'mongodb' as const, port: 27017, tls: true }
    expect(connectionUrl(mongo)).toBe('mongodb://app@db.internal:27017/shop?tls=true')
    expect(parseConnectionUrl(connectionUrl(mongo))?.changes).toMatchObject({
      host: 'db.internal',
      tls: true,
    })
    expect(connectionUrl({ ...mongo, srv: true, host: 'cluster.example.net' })).toBe(
      'mongodb+srv://app@cluster.example.net/shop',
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
