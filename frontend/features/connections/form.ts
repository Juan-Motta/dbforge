import {
  DEFAULT_PORTS,
  isFileEngine,
  KEY_ENGINES,
  TLS_BY_DEFAULT,
  TLS_ENGINES,
  type Connection,
  type Engine,
  type Environment,
} from '../../domain/connection'
import type { ConnectionInput } from './api'

/** Form values as typed by the user. */
export interface ConnectionForm {
  name: string
  engine: Engine
  environment: Environment | null
  host: string
  port: string
  database: string
  username: string
  password: string
  tls: boolean
  allowSelfSigned: boolean
  srv: boolean
}

export function emptyForm(): ConnectionForm {
  return {
    name: '',
    engine: 'postgres',
    environment: null,
    host: 'localhost',
    port: String(DEFAULT_PORTS.postgres),
    database: '',
    username: '',
    password: '',
    tls: false,
    allowSelfSigned: false,
    srv: false,
  }
}

/** A saved connection as form values. A host saved as a whole URL is split into fields. */
export function formFromConnection(connection: Connection): ConnectionForm {
  const form = { ...connection, port: String(connection.port), password: '' }
  const parsed = parseConnectionUrl(connection.host)
  return parsed ? { ...form, ...parsed.changes } : form
}

/** Switches engine, moving the port and TLS to the new defaults unless the user changed them. */
export function withEngine(form: ConnectionForm, engine: Engine): ConnectionForm {
  const usesDefaultPort = form.port === String(DEFAULT_PORTS[form.engine])
  const usesDefaultTls = form.tls === TLS_BY_DEFAULT.includes(form.engine)
  return {
    ...form,
    engine,
    port: usesDefaultPort ? String(DEFAULT_PORTS[engine]) : form.port,
    tls: usesDefaultTls ? TLS_BY_DEFAULT.includes(engine) : form.tls,
  }
}

export function isValidPort(port: string): boolean {
  const value = Number(port)
  return Number.isInteger(value) && value > 0 && value <= 65535
}

/** Enough to try connecting: a file for SQLite, a valid port for servers. */
export function canTest(form: ConnectionForm): boolean {
  if (isFileEngine(form.engine)) return form.database.trim() !== ''
  // A seed list's DNS records name the port.
  return usesSrv(form) || isValidPort(form.port)
}

function usesSrv(form: ConnectionForm): boolean {
  return form.engine === 'mongodb' && form.srv
}

export function canSubmit(form: ConnectionForm): boolean {
  return form.name.trim() !== '' && canTest(form)
}

/** An empty password means "no change", so it is left out. A file needs no server details. */
export function toInput(form: ConnectionForm, id?: string): ConnectionInput {
  const { password, port, ...rest } = form
  const off = { tls: false, allowSelfSigned: false, srv: false }
  if (isFileEngine(form.engine)) {
    return { ...rest, ...off, id, host: '', port: 0, username: '', password: undefined }
  }
  const srv = usesSrv(form)
  const tls = TLS_ENGINES.includes(form.engine) && (form.tls || srv)
  return {
    ...rest,
    id,
    port: srv && !isValidPort(port) ? DEFAULT_PORTS.mongodb : Number(port),
    // Redis databases are numbered and all listed in the explorer; there is none to pick here.
    database: form.engine === 'redis' ? '' : form.database,
    // An account key stands alone.
    username: KEY_ENGINES.includes(form.engine) ? '' : form.username,
    password: password === '' ? undefined : password,
    tls,
    allowSelfSigned: tls && form.allowSelfSigned,
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
 * silently. `null` for anything else.
 */
export function parseConnectionUrl(
  text: string,
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
  const ignored = others.length > 0 ? [`hosts ${others.join(',')}`] : []
  const params = new URLSearchParams(query)
  const on = (...names: string[]) => names.some((name) => params.get(name) === 'true')

  if (redis) {
    ignored.push(...[...params].map(([key, value]) => `${key}=${value}`))
    if (fragment && fragment !== 'insecure') ignored.push(`#${fragment}`)
    const tls = scheme === 'rediss'
    return {
      changes: {
        engine: 'redis',
        host: decode(host),
        port: port || String(DEFAULT_PORTS.redis),
        username: decode(user),
        password: decode(pass),
        database: decode(path),
        tls,
        allowSelfSigned: tls && fragment === 'insecure',
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
      port: port || String(DEFAULT_PORTS.mongodb),
      username: decode(user),
      password: decode(pass),
      database: decode(path),
      tls,
      allowSelfSigned: tls && on('tlsAllowInvalidCertificates', 'tlsInsecure'),
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
  const endpoint = settings.get('accountendpoint')
  if (!endpoint || !URL.canParse(endpoint)) return null
  const url = new URL(endpoint)
  const tls = url.protocol === 'https:'
  return {
    changes: {
      engine: 'cosmosdb',
      host: url.hostname,
      port: url.port || (tls ? '443' : '80'),
      username: '',
      password: settings.get('accountkey') ?? '',
      tls,
      allowSelfSigned: false,
      srv: false,
    },
    ignored,
  }
}
