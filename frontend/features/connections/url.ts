import type { Connection } from '../../domain/connection'

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
      const scheme = connection.tls ? 'rediss' : 'redis'
      const insecure = connection.tls && connection.allowSelfSigned ? '#insecure' : ''
      return `${scheme}://${user}${host}:${port}${path}${insecure}`
    }
    case 'mongodb': {
      if (/^mongodb(\+srv)?:\/\//.test(host)) return withoutPassword(host)
      const options = new URLSearchParams()
      if (connection.tls && !connection.srv) options.set('tls', 'true')
      if (connection.tls && connection.allowSelfSigned) options.set('tlsAllowInvalidCertificates', 'true')
      const query = options.size > 0 ? `?${options}` : ''
      return connection.srv
        ? `mongodb+srv://${user}${host}/${encodeURIComponent(database)}${query}`
        : `mongodb://${user}${host}:${port}/${encodeURIComponent(database)}${query}`
    }
    case 'mariadb':
    case 'mysql':
    case 'postgres': {
      const scheme = engine === 'postgres' ? 'postgresql' : engine
      return `${scheme}://${user}${host}:${port}${path}`
    }
  }
}

/** A URL saved whole as the host, with any password taken out. */
function withoutPassword(url: string): string {
  return url.replace(/^([a-z+]+:\/\/)([^:@/]*):[^@/]*@/i, (_, scheme: string, user: string) =>
    user ? `${scheme}${user}@` : scheme,
  )
}
