import { encrypts, type Connection, type SslMode } from '../../domain/connection'

/**
 * A URL (or, for Cosmos DB, a connection string) that points at the
 * connection, without its password or key, in a form DBForge's connection
 * dialog reads back when pasted. SQLite gives its file.
 */
export function connectionUrl(connection: Connection): string {
  const { engine, host, port, username, database } = connection
  const user = username ? `${encodeURIComponent(username)}@` : ''
  const path = database ? `/${encodeURIComponent(database)}` : ''
  switch (engine) {
    case 'sqlite':
      return database
    case 'cosmosdb':
      return `AccountEndpoint=${host};`
    case 'sqlserver': {
      const parts = [`sqlserver://${host}:${port}`]
      if (database) parts.push(`database=${database}`)
      if (username) parts.push(`user=${username}`)
      return parts.join(';')
    }
    case 'redis': {
      if (/^rediss?:\/\//.test(host)) return withoutPassword(host)
      const { mode } = connection.ssl
      const scheme = encrypts(mode) ? 'rediss' : 'redis'
      const insecure = mode === 'require' ? '#insecure' : ''
      return `${scheme}://${user}${host}:${port}${path}${insecure}`
    }
    case 'mongodb': {
      if (/^mongodb(\+srv)?:\/\//.test(host)) return withoutPassword(host)
      const options = new URLSearchParams()
      const { mode } = connection.ssl
      if (encrypts(mode) && !connection.srv) options.set('tls', 'true')
      if (mode === 'require') options.set('tlsAllowInvalidCertificates', 'true')
      const query = options.size > 0 ? `?${options}` : ''
      return connection.srv
        ? `mongodb+srv://${user}${host}/${encodeURIComponent(database)}${query}`
        : `mongodb://${user}${host}:${port}/${encodeURIComponent(database)}${query}`
    }
    case 'mariadb':
    case 'mysql':
    case 'postgres': {
      const scheme = engine === 'postgres' ? 'postgresql' : engine
      return `${scheme}://${user}${host}:${port}${path}${sslQuery(engine, connection.ssl.mode)}`
    }
  }
}

/** MySQL's names for the SSL modes, as its `ssl-mode` option takes them. */
const MYSQL_SSL_MODES: Record<SslMode, string> = {
  disable: 'DISABLED',
  prefer: 'PREFERRED',
  require: 'REQUIRED',
  'verify-ca': 'VERIFY_CA',
  'verify-full': 'VERIFY_IDENTITY',
}

/**
 * The SSL mode as a URL option, left out when it is `prefer`, the drivers'
 * default. Certificate files and the SSH tunnel are this machine's, never in the URL.
 */
function sslQuery(engine: 'postgres' | 'mysql' | 'mariadb', mode: SslMode): string {
  if (mode === 'prefer') return ''
  return engine === 'postgres' ? `?sslmode=${mode}` : `?ssl-mode=${MYSQL_SSL_MODES[mode]}`
}

/** A URL saved whole as the host, with any password taken out. */
function withoutPassword(url: string): string {
  return url.replace(/^([a-z+]+:\/\/)([^:@/]*):[^@/]*@/i, (_, scheme: string, user: string) =>
    user ? `${scheme}${user}@` : scheme,
  )
}
