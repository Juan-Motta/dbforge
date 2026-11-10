// An in-memory Redis for `pnpm dev` in a plain browser, so the key browser can
// be developed without a server. It mirrors the backend's replies, not Redis.
import type { Entry, KeyEdit, KeyInfo, KeyValue, NewKey, Overview } from '../features/keyspace/api'

type Value =
  | { kind: 'string'; value: string; hex?: string }
  | { kind: 'hash'; value: Map<string, string> }
  | { kind: 'list'; value: string[] }
  | { kind: 'set'; value: Set<string> }
  | { kind: 'zset'; value: Map<string, number> }
  | { kind: 'stream'; value: [string, Record<string, string>][] }

interface Stored {
  data: Value
  expiresAt: number | null
}

const store = new Map<string, Stored>()
const put = (name: string, data: Value, ttlSeconds?: number) =>
  store.set(name, { data, expiresAt: ttlSeconds ? Date.now() + ttlSeconds * 1000 : null })

const ids = ['7f3a9c21e8', '7f39b02d11', '7f2e88aa04', '7f1b5e6c70', '7f0aa1d2b9', '7ef8c3b551', '7ee41f0a87']
ids.forEach((id, i) =>
  put(
    `session:${id}`,
    {
      kind: 'hash',
      value: new Map([
        ['user_id', `u_${88412 + i}`],
        ['plan', i % 3 === 0 ? 'pro' : 'free'],
        ['csrf', 'b8e1f0c2a94d4e2f'],
        ['created_at', String(1730718422 + i * 60)],
        ['ip', `203.0.113.${42 + i}`],
        [
          'user_agent',
          'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15',
        ],
      ]),
    },
    i === 5 ? undefined : 600 + i * 900,
  ),
)
put('session:7f2c1d9f3e:csrf', { kind: 'string', value: 'b8e1f0c2a94d4e2f' }, 3480)
put('session:7f1b5e6c70:roles', { kind: 'set', value: new Set(['admin', 'billing', 'viewer']) }, 720)
put(
  'session:7f0aa1d2b9:events',
  { kind: 'list', value: ['login', 'view:/orders', 'export:csv', 'logout'] },
  14400,
)
put('session:active_by_ts', { kind: 'zset', value: new Map(ids.map((id, i) => [id, 1730718422 + i * 60])) })
put('session:audit', {
  kind: 'stream',
  value: [
    ['1730718422000-0', { event: 'login', user: 'u_88412' }],
    ['1730718482000-0', { event: 'export', user: 'u_88412', format: 'csv' }],
  ],
})
for (let i = 1; i <= 24; i++)
  put(`cart:${1000 + i}`, { kind: 'string', value: JSON.stringify({ items: i % 4, total: i * 12.5 }) })
put('leaderboard:weekly', {
  kind: 'zset',
  value: new Map([
    ['ana', 980],
    ['bo', 1210],
    ['cy', 640],
  ]),
})
put('queue:emails', { kind: 'list', value: ['{"to":"ana@example.com","template":"welcome"}'] })
// {"theme": "dark", "lang": "es", "beta": true} in MessagePack, which is binary.
put('user:1:prefs', {
  kind: 'string',
  value: '',
  hex: '83a57468656d65a46461726ba46c616e67a26573a462657461c3',
})
put('feature_flags', {
  kind: 'hash',
  value: new Map([
    ['new_checkout', 'true'],
    ['dark_mode', 'false'],
  ]),
})

const ttlOf = (stored: Stored) =>
  stored.expiresAt === null ? null : Math.max(stored.expiresAt - Date.now(), 0)
const glob = (pattern: string) =>
  new RegExp(
    '^' +
      (pattern || '*')
        .replace(/[.+^${}()|\\]/g, '\\$&')
        .replace(/\*/g, '.*')
        .replace(/\?/g, '.') +
      '$',
  )
const bytes = (text: string) => new TextEncoder().encode(text).length
const text = (value: string) => ({ text: value, binary: false })

export function overview(): Overview {
  return {
    version: '7.4.1 (mock)',
    mode: 'standalone',
    keys: store.size,
    usedMemory: 412 * 1024 * 1024,
    maxMemory: 1024 * 1024 * 1024,
    opsPerSec: 3240,
    keyspaceHits: 974,
    keyspaceMisses: 26,
    clients: 58,
    evictedKeys: 0,
    expiredKeys: 18204,
  }
}

export function scan(pattern: string, kind: string | null, cursor: string, limit: number) {
  const matcher = glob(pattern)
  const all = [...store]
    .filter(([name, stored]) => matcher.test(name) && (!kind || stored.data.kind === kind))
    .sort(([a], [b]) => (a < b ? -1 : 1))
  const start = Number(cursor)
  const keys: KeyInfo[] = all
    .slice(start, start + limit)
    .map(([name, stored]) => ({ name, binary: false, kind: stored.data.kind, ttlMs: ttlOf(stored) }))
  return { keys, cursor: start + limit >= all.length ? '0' : String(start + limit) }
}

function find(name: string): Stored {
  const stored = store.get(name)
  if (!stored) throw `The key ${name} no longer exists.`
  return stored
}

export function readKey(name: string, filter: string): KeyValue {
  const stored = find(name)
  const { data } = stored
  const keep = (s: string) => !filter || s.includes(filter)
  let entries: Entry[]
  let length: number
  switch (data.kind) {
    case 'string':
      length = data.hex ? data.hex.length / 2 : bytes(data.value)
      entries = [
        {
          key: null,
          value: data.hex ? { text: data.hex, binary: true } : text(data.value),
          score: null,
          size: length,
        },
      ]
      break
    case 'hash':
      length = data.value.size
      entries = [...data.value]
        .filter(([f]) => keep(f))
        .map(([f, v]) => ({ key: text(f), value: text(v), score: null, size: bytes(v) }))
      break
    case 'list':
      length = data.value.length
      entries = data.value.map((v, i) => ({
        key: text(String(i)),
        value: text(v),
        score: null,
        size: bytes(v),
      }))
      break
    case 'set':
      length = data.value.size
      entries = [...data.value]
        .filter(keep)
        .sort()
        .map((m) => ({ key: null, value: text(m), score: null, size: bytes(m) }))
      break
    case 'zset':
      length = data.value.size
      entries = [...data.value]
        .filter(([m]) => keep(m))
        .sort(([, a], [, b]) => a - b)
        .map(([m, score]) => ({ key: null, value: text(m), score, size: bytes(m) }))
      break
    case 'stream':
      length = data.value.length
      entries = data.value.map(([id, fields]) => ({
        key: text(id),
        value: text(JSON.stringify(fields)),
        score: null,
        size: bytes(JSON.stringify(fields)),
      }))
      break
  }
  return {
    name,
    binary: false,
    kind: data.kind,
    ttlMs: ttlOf(stored),
    length,
    bytes: 412,
    encoding: 'listpack',
    entries,
    next: null,
  }
}

export function editKey(name: string, kind: string, edit: KeyEdit) {
  const stored = find(name)
  const { data } = stored
  if (data.kind !== kind) throw `The key ${name} is now a ${data.kind}, not a ${kind}. Reload it.`
  switch (edit.kind) {
    case 'setString':
      if (data.kind === 'string') data.value = edit.value
      break
    case 'setField':
      if (data.kind === 'hash') data.value.set(edit.field.text, edit.value)
      break
    case 'removeField':
      if (data.kind === 'hash') data.value.delete(edit.field.text)
      break
    case 'setItem':
      if (data.kind === 'list') data.value[edit.index] = edit.value
      break
    case 'pushItem':
      if (data.kind === 'list') {
        if (edit.head) data.value.unshift(edit.value)
        else data.value.push(edit.value)
      }
      break
    case 'removeItem':
      if (data.kind === 'list') data.value.splice(edit.index, 1)
      break
    case 'addMember':
      if (data.kind === 'set') data.value.add(edit.member.text)
      if (data.kind === 'zset') data.value.set(edit.member.text, edit.score ?? 0)
      break
    case 'removeMember':
      if (data.kind === 'set' || data.kind === 'zset') data.value.delete(edit.member.text)
      break
    case 'addEntry':
      if (data.kind === 'stream') data.value.push([`${Date.now()}-0`, Object.fromEntries(edit.fields)])
      break
    case 'removeEntry':
      if (data.kind === 'stream') data.value = data.value.filter(([id]) => id !== edit.id)
      break
    case 'expire':
      stored.expiresAt = edit.ttlMs === null ? null : Date.now() + edit.ttlMs
      break
    case 'rename':
      if (store.has(edit.to)) throw `A key named ${edit.to} already exists.`
      store.delete(name)
      store.set(edit.to, stored)
      break
    case 'delete':
      store.delete(name)
      break
  }
}

export function createKey(key: NewKey) {
  if (store.has(key.name)) throw `A key named ${key.name} already exists.`
  const field = key.field ?? 'field'
  const data: Value =
    key.kind === 'hash'
      ? { kind: 'hash', value: new Map([[field, key.value]]) }
      : key.kind === 'list'
        ? { kind: 'list', value: [key.value] }
        : key.kind === 'set'
          ? { kind: 'set', value: new Set([key.value]) }
          : key.kind === 'zset'
            ? { kind: 'zset', value: new Map([[key.value, key.score ?? 0]]) }
            : key.kind === 'stream'
              ? { kind: 'stream', value: [[`${Date.now()}-0`, { [field]: key.value }]] }
              : { kind: 'string', value: key.value }
  store.set(key.name, { data, expiresAt: key.ttlMs ? Date.now() + key.ttlMs : null })
}

/** A few commands, replied the way the backend formats redis-cli output. */
export function cli(command: string): string[] {
  const [name = '', ...args] = command.trim().split(/\s+/)
  const key = args[0] ?? ''
  const stored = store.get(key)
  switch (name.toUpperCase()) {
    case 'PING':
      return ['PONG']
    case 'DBSIZE':
      return [`(integer) ${store.size}`]
    case 'TTL':
      return [`(integer) ${stored ? Math.round((ttlOf(stored) ?? -1000) / 1000) : -2}`]
    case 'GET':
      if (!stored) return ['(nil)']
      return stored.data.kind === 'string'
        ? [`"${stored.data.value.replace(/"/g, '\\"')}"`]
        : ['(error) WRONGTYPE Operation against a key holding the wrong kind of value']
    case 'HGET':
      if (stored?.data.kind !== 'hash') return ['(nil)']
      return stored.data.value.has(args[1]) ? [`"${stored.data.value.get(args[1])}"`] : ['(nil)']
    case 'HGETALL':
      if (stored?.data.kind !== 'hash') return ['(empty array)']
      return [...stored.data.value].flat().map((v, i) => `${String(i + 1).padStart(2)}) "${v}"`)
    case 'MEMORY':
      return ['(integer) 412']
    default:
      return [`(error) ERR unknown command '${name}' (the mock knows PING, DBSIZE, TTL, GET, HGET, HGETALL)`]
  }
}
