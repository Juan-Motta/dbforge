import { connectionAddress, isFileEngine, TLS_ENGINES, type Connection } from '../../domain/connection'

/**
 * What the connection popover lists, as label/value pairs: the server (`null`
 * while it is being asked), where it is, and TLS where the engine can use it.
 */
export function connectionFacts(connection: Connection, server: string | null): [string, string][] {
  const facts: [string, string][] = [['Server', server ?? 'Checking…']]
  if (isFileEngine(connection.engine)) return [...facts, ['File', connection.database]]
  facts.push(['Address', connectionAddress(connection)])
  if (connection.database) facts.push(['Database', connection.database])
  if (TLS_ENGINES.includes(connection.engine)) {
    const tls = connection.tls || connection.srv
    facts.push([
      'TLS',
      !tls ? 'Off' : connection.allowSelfSigned ? 'On · self-signed certificate accepted' : 'On',
    ])
  }
  return facts
}
