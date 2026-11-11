import {
  connectionAddress,
  ENGINES,
  isFileEngine,
  type Connection,
  type SslMode,
} from '../../domain/connection'
import type { SharedMessages } from '../../lib/messages'
import type { ShellMessages } from './messages'

/**
 * What the connection popover lists, as label/value pairs: the server (`null`
 * while it is being asked), where it is, its SSL mode where the engine has SSL,
 * and the SSH tunnel it goes through.
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
  const spec = ENGINES[connection.engine]
  if (spec.ssl.modes.length > 1) facts.push([m.ssl, sslModeLabel(connection.ssl.mode, m)])
  const { ssh } = connection
  if (spec.tunnels && ssh.enabled) facts.push([m.ssh, `${ssh.username}@${ssh.host}:${ssh.port}`])
  return facts
}

function sslModeLabel(mode: SslMode, m: ShellMessages): string {
  const labels: Record<SslMode, string> = {
    disable: m.sslDisable,
    prefer: m.sslPrefer,
    require: m.sslRequire,
    'verify-ca': m.sslVerifyCa,
    'verify-full': m.sslVerifyFull,
  }
  return labels[mode]
}
