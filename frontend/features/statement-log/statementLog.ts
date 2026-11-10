import type { Engine } from '../../domain/connection'
import type { Origin, StatementEntry } from './api'

/** As many as the backend keeps. */
export const CAPACITY = 2000

/** The console's filter: what the statements were for, grouped as people think of them. */
export type Kind = 'all' | 'console' | 'agents' | 'changes' | 'reads' | 'metadata'

export const KINDS: Kind[] = ['all', 'console', 'agents', 'changes', 'reads', 'metadata']

const KIND_OF: Record<Origin, Exclude<Kind, 'all'>> = {
  console: 'console',
  edit: 'changes',
  admin: 'changes',
  dump: 'changes',
  browse: 'reads',
  export: 'reads',
  metadata: 'metadata',
  connect: 'metadata',
  agent: 'agents',
}

export function kindOf(origin: Origin): Exclude<Kind, 'all'> {
  return KIND_OF[origin]
}

/**
 * `entries` with `incoming` added, by id: the list read at start and the
 * events that arrive meanwhile may overlap. Oldest first, at most `capacity`.
 */
export function merge(entries: StatementEntry[], incoming: StatementEntry[], capacity = CAPACITY) {
  const known = new Set(entries.map((entry) => entry.id))
  const added = incoming.filter((entry) => !known.has(entry.id))
  if (added.length === 0) return entries
  const all = [...entries, ...added].sort((a, b) => a.id - b.id)
  return all.length > capacity ? all.slice(all.length - capacity) : all
}

export function visible(entries: StatementEntry[], kind: Kind): StatementEntry[] {
  return kind === 'all' ? entries : entries.filter((entry) => kindOf(entry.origin) === kind)
}

const SQL_ENGINES: ReadonlySet<Engine> = new Set(['postgres', 'mysql', 'mariadb', 'sqlite', 'sqlserver'])

/** The statement as it is pasted into a console: SQL ends with `;`, other languages as they are. */
export function runnable(entry: StatementEntry, engine: Engine | undefined): string {
  const text = entry.text.trim()
  return engine && SQL_ENGINES.has(engine) && !text.endsWith(';') ? `${text};` : text
}

/** The statements as one script, in the order they ran. */
export function script(entries: StatementEntry[], engineOf: (connectionId: string) => Engine | undefined) {
  return entries.map((entry) => runnable(entry, engineOf(entry.connectionId))).join('\n\n')
}

/** `14:03:27.512`, in local time. */
export function clockTime(at: number): string {
  const date = new Date(at)
  const pad = (value: number, width = 2) => String(value).padStart(width, '0')
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(), 3)}`
}

/** `8 ms`, `1.24 s`. */
export function duration(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2)} s`
}
