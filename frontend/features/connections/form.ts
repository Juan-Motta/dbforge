import {
  defaultSslMode,
  encrypts,
  ENGINES,
  isFileEngine,
  NO_SSH_TUNNEL,
  sslModeWorksThroughSsh,
  type AgentAccess,
  type Connection,
  type Engine,
  type Ssl,
  type SslMode,
  type SshTunnel,
} from '../../domain/connection'
import type { ConnectionInput } from './api'
import { messages, type ConnectionMessages } from './messages'

/** Form values as typed by the user. */
export interface ConnectionForm {
  name: string
  engine: Engine
  /** A tag id, or none. */
  environment: string | null
  /** The server's name, or for an account engine its whole URI (`https://….documents.azure.com:443/`). */
  host: string
  port: string
  database: string
  username: string
  password: string
  /**
   * How SSL/TLS is used. For an account engine the URI's scheme decides whether
   * it encrypts, and `require` stands for its "accept self-signed" checkbox.
   */
  ssl: Ssl
  ssh: SshForm
  /** The SSH password or key passphrase; empty keeps the saved one. */
  sshSecret: string
  srv: boolean
  agentAccess: AgentAccess
}

/** The SSH tunnel as typed: its port is text, like the server's. */
export interface SshForm extends Omit<SshTunnel, 'port'> {
  port: string
}

/** SSL settings with `mode` and no files. */
export function sslWith(mode: SslMode): Ssl {
  return { mode, caFile: '', certFile: '', keyFile: '' }
}

const NO_SSH_FORM: SshForm = { ...NO_SSH_TUNNEL, port: String(NO_SSH_TUNNEL.port) }

export function emptyForm(): ConnectionForm {
  return {
    name: '',
    engine: 'postgres',
    environment: null,
    host: 'localhost',
    port: String(ENGINES.postgres.defaultPort),
    database: '',
    username: '',
    password: '',
    ssl: sslWith(defaultSslMode('postgres')),
    ssh: NO_SSH_FORM,
    sshSecret: '',
    srv: false,
    agentAccess: 'none',
  }
}

/** A saved connection as form values. A host saved as a whole URL is split into fields. */
export function formFromConnection(connection: Connection): ConnectionForm {
  const { id: _id, ...fields } = connection
  const form: ConnectionForm = {
    ...fields,
    port: String(connection.port),
    password: '',
    ssh: { ...connection.ssh, port: String(connection.ssh.port) },
    sshSecret: '',
  }
  if (isAccount(connection.engine)) {
    const scheme = encrypts(connection.ssl.mode) ? 'https' : 'http'
    return { ...form, host: `${scheme}://${connection.host}:${connection.port}/` }
  }
  const parsed = parseConnectionUrl(connection.host)
  return parsed ? { ...form, ...parsed.changes } : form
}

/**
 * Switches engine, moving the port to the new default unless the user changed it.
 * An account engine's host is a URI, which no other engine's host field takes.
 * The SSL mode stays when the new engine has it, else becomes its default.
 */
export function withEngine(form: ConnectionForm, engine: Engine): ConnectionForm {
  const usesDefaultPort = form.port === String(ENGINES[form.engine].defaultPort)
  const host = isAccount(engine) === isAccount(form.engine) ? form.host : isAccount(engine) ? '' : 'localhost'
  const mode = ENGINES[engine].ssl.modes.includes(form.ssl.mode) ? form.ssl.mode : defaultSslMode(engine)
  return {
    ...form,
    engine,
    host,
    port: usesDefaultPort ? String(ENGINES[engine].defaultPort) : form.port,
    ssl: { ...form.ssl, mode },
  }
}

/** The SSL modes the form offers: an engine's, without `disable` for a seed list, which is always encrypted. */
export function sslModesOf(form: ConnectionForm): SslMode[] {
  const modes = ENGINES[form.engine].ssl.modes
  return usesSrv(form) ? modes.filter(encrypts) : modes
}

/** The tunnel is on, where the engine can use one. */
export function tunnelOn(form: ConnectionForm): boolean {
  return ENGINES[form.engine].tunnels && form.ssh.enabled
}

/** The SSL mode checks the server's name, which this engine can't do through the tunnel. */
export function sslBlockedByTunnel(form: ConnectionForm): boolean {
  return tunnelOn(form) && !sslModeWorksThroughSsh(form.engine, effectiveSslMode(form))
}

/** A tunnel reaches one host and port, not a DNS seed list or a whole URL. */
export function tunnelUnreachable(form: ConnectionForm): boolean {
  return tunnelOn(form) && (usesSrv(form) || form.host.includes('://'))
}

/** A client certificate without its key, or a key without its certificate, where they are separate files. */
export function clientCertIncomplete(form: ConnectionForm): boolean {
  const support = ENGINES[form.engine].ssl
  if (!support.clientCert || !support.separateKey || !encrypts(form.ssl.mode)) return false
  return (form.ssl.certFile.trim() === '') !== (form.ssl.keyFile.trim() === '')
}

/** Enough to open the tunnel: its host, user and port, and the key file for key auth. */
function tunnelComplete(ssh: SshForm): boolean {
  return (
    ssh.host.trim() !== '' &&
    ssh.username.trim() !== '' &&
    isValidPort(ssh.port) &&
    (ssh.auth !== 'key' || ssh.keyFile.trim() !== '')
  )
}

/** The mode sent for a server: a seed list is always encrypted, as mongodb+srv:// defines. */
function effectiveSslMode(form: ConnectionForm): SslMode {
  return usesSrv(form) && !encrypts(form.ssl.mode) ? 'verify-full' : form.ssl.mode
}

function isAccount(engine: Engine): boolean {
  return ENGINES[engine].reach === 'account'
}

/** An account URI: `https://<account>.documents.azure.com:443/`, or `http://` for the local emulator. */
function accountUrl(text: string): URL | null {
  const trimmed = text.trim()
  if (!/^https?:\/\/[^/]/i.test(trimmed) || !URL.canParse(trimmed)) return null
  return new URL(trimmed)
}

export function isValidPort(port: string): boolean {
  const value = Number(port)
  return Number.isInteger(value) && value > 0 && value <= 65535
}

/**
 * Enough to try connecting: a file for SQLite, a valid port for servers, and
 * SSL and tunnel settings that work together.
 */
export function canTest(form: ConnectionForm): boolean {
  if (isFileEngine(form.engine)) return form.database.trim() !== ''
  if (isAccount(form.engine)) return accountUrl(form.host) !== null
  if (clientCertIncomplete(form)) return false
  if (tunnelOn(form) && (!tunnelComplete(form.ssh) || tunnelUnreachable(form) || sslBlockedByTunnel(form))) {
    return false
  }
  // A seed list's DNS records name the port.
  return usesSrv(form) || isValidPort(form.port)
}

function usesSrv(form: ConnectionForm): boolean {
  return form.engine === 'mongodb' && form.srv
}

export function canSubmit(form: ConnectionForm): boolean {
  return form.name.trim() !== '' && canTest(form)
}

/**
 * An empty password or SSH secret means "no change", so it is left out. A file
 * needs no server details, SSL or tunnel; an account has no tunnel.
 */
export function toInput(form: ConnectionForm, id?: string): ConnectionInput {
  const { password, port, sshSecret, ssh: sshForm, ...rest } = form
  const secret = (value: string) => (value === '' ? undefined : value)
  const ssh: SshTunnel = ENGINES[form.engine].tunnels
    ? { ...sshForm, port: isValidPort(sshForm.port) ? Number(sshForm.port) : NO_SSH_TUNNEL.port }
    : NO_SSH_TUNNEL
  const off = { ssl: sslWith('disable'), ssh: NO_SSH_TUNNEL, sshSecret: undefined, srv: false }
  if (isFileEngine(form.engine)) {
    return { ...rest, ...off, id, host: '', port: 0, username: '', password: undefined }
  }
  const account = isAccount(form.engine) ? accountUrl(form.host) : null
  if (account) {
    // The databases are all listed in the explorer, and the key stands alone.
    const https = account.protocol === 'https:'
    return {
      ...rest,
      ...off,
      id,
      host: account.hostname,
      port: Number(account.port || (https ? 443 : 80)),
      database: '',
      username: '',
      password: secret(password),
      ssl: sslWith(!https ? 'disable' : form.ssl.mode === 'require' ? 'require' : 'verify-full'),
    }
  }
  const srv = usesSrv(form)
  return {
    ...rest,
    id,
    port: srv && !isValidPort(port) ? ENGINES.mongodb.defaultPort : Number(port),
    // Redis databases are numbered and all listed in the explorer; there is none to pick here.
    database: form.engine === 'redis' ? '' : form.database,
    password: secret(password),
    ssl: { ...form.ssl, mode: effectiveSslMode(form) },
    ssh,
    sshSecret: secret(sshSecret),
    srv,
  }
}

const URL_PATTERN =
  /^(redis|rediss|mongodb|mongodb\+srv):\/\/(?:([^:@/]*)(?::([^@/]*))?@)?([^/?#]*)(?:\/([^?#]*))?(?:\?([^#]*))?(?:#(.*))?$/

/** MongoDB URL options the form has fields for; `authSource=admin` is what it uses anyway. */
const MONGO_OPTIONS = ['tls', 'ssl', 'tlsAllowInvalidCertificates', 'tlsInsecure']

/**
 * The form values a `redis://`, `rediss://`, `mongodb://` or `mongodb+srv://`
 * URL, or a Cosmos DB connection string, stands for, and the parts of it the
 * form can't hold (`ignored`), so they can be shown rather than dropped
 * silently, in the language of `m`. `null` for anything else.
 */
export function parseConnectionUrl(
  text: string,
  m: ConnectionMessages = messages.en,
): { changes: Partial<ConnectionForm>; ignored: string[] } | null {
  const cosmos = parseCosmosConnectionString(text)
  if (cosmos) return cosmos
  const match = URL_PATTERN.exec(text.trim())
  if (!match) return null
  const [, scheme, user = '', pass = '', hosts, path = '', query = '', fragment = ''] = match
  const decode = (part: string) => {
    try {
      return decodeURIComponent(part)
    } catch {
      return part
    }
  }
  const redis = scheme.startsWith('redis')
  const [first, ...others] = hosts.split(',')
  const cut = first.lastIndexOf(':')
  const host = cut > 0 && !first.endsWith(']') ? first.slice(0, cut) : first
  const port = cut > 0 && !first.endsWith(']') ? first.slice(cut + 1) : ''
  const ignored = others.length > 0 ? [m.extraHosts(others.join(','))] : []
  const params = new URLSearchParams(query)
  const on = (...names: string[]) => names.some((name) => params.get(name) === 'true')

  if (redis) {
    ignored.push(...[...params].map(([key, value]) => `${key}=${value}`))
    if (fragment && fragment !== 'insecure') ignored.push(`#${fragment}`)
    return {
      changes: {
        engine: 'redis',
        host: decode(host),
        port: port || String(ENGINES.redis.defaultPort),
        username: decode(user),
        password: decode(pass),
        database: decode(path),
        ssl: sslWith(scheme !== 'rediss' ? 'disable' : fragment === 'insecure' ? 'require' : 'verify-full'),
        srv: false,
      },
      ignored,
    }
  }

  for (const [key, value] of params) {
    if (MONGO_OPTIONS.includes(key) || (key === 'authSource' && value === 'admin')) continue
    ignored.push(`${key}=${value}`)
  }
  const srv = scheme === 'mongodb+srv'
  const tls = srv || on('tls', 'ssl')
  return {
    changes: {
      engine: 'mongodb',
      host: decode(host),
      port: port || String(ENGINES.mongodb.defaultPort),
      username: decode(user),
      password: decode(pass),
      database: decode(path),
      ssl: sslWith(
        !tls ? 'disable' : on('tlsAllowInvalidCertificates', 'tlsInsecure') ? 'require' : 'verify-full',
      ),
      srv,
    },
    ignored,
  }
}

/**
 * `AccountEndpoint=https://<account>.documents.azure.com:443/;AccountKey=…;`, as
 * the Azure portal and the emulator give it. Its key is the account key.
 */
function parseCosmosConnectionString(
  text: string,
): { changes: Partial<ConnectionForm>; ignored: string[] } | null {
  const settings = new Map<string, string>()
  const ignored: string[] = []
  for (const part of text
    .trim()
    .split(';')
    .map((p) => p.trim())) {
    // A key ends in Base64 padding, so only the first `=` splits a setting.
    const cut = part.indexOf('=')
    if (cut <= 0) continue
    const name = part.slice(0, cut).toLowerCase()
    if (name === 'accountendpoint' || name === 'accountkey') settings.set(name, part.slice(cut + 1))
    else ignored.push(part)
  }
  const endpoint = accountUrl(settings.get('accountendpoint') ?? '')
  if (!endpoint) return null
  return {
    changes: {
      engine: 'cosmosdb',
      host: endpoint.href,
      password: settings.get('accountkey') ?? '',
      username: '',
      srv: false,
    },
    ignored,
  }
}
