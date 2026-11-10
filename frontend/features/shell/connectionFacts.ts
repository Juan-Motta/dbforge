import { connectionAddress, isFileEngine, TLS_ENGINES, type Connection } from '../../domain/connection'
import type { SharedMessages } from '../../lib/messages'
import type { ShellMessages } from './messages'

/**
 * What the connection popover lists, as label/value pairs: the server (`null`
 * while it is being asked), where it is, and TLS where the engine can use it.
 */
export function connectionFacts(
  connection: Connection,
  server: string | null,
  m: ShellMessages,
  shared: SharedMessages,
): [string, string][] {
  const facts: [string, string][] = [[m.server, server ?? m.checking]]
  if (isFileEngine(connection.engine)) return [...facts, [m.file, connection.database]]
  facts.push([m.address, connectionAddress(connection, shared)])
  if (connection.database) facts.push([m.database, connection.database])
  if (TLS_ENGINES.includes(connection.engine)) {
    const tls = connection.tls || connection.srv
    facts.push([m.tls, !tls ? m.tlsOff : connection.allowSelfSigned ? m.tlsSelfSigned : m.tlsOn])
  }
  return facts
}
