import { invoke } from '@tauri-apps/api/core'

/** `string`, `hash`, `list`, `set`, `zset`, `stream`, or a module's type. */
export type KeyKind = string

export interface KeyInfo {
  name: string
  kind: KeyKind
  /** Milliseconds left; `null` when the key never expires. */
  ttlMs: number | null
}

export interface ScanPage {
  keys: KeyInfo[]
  /** Where the next page starts; `"0"` when the scan is complete. */
  cursor: string
}

/** Bytes for display: the text when it is UTF-8, else hex. */
export interface Text {
  text: string
  binary: boolean
}

export interface Entry {
  /** Hash field, list index or stream entry id; `null` for sets and strings. */
  key: Text | null
  /** A stream entry's fields are a JSON object. */
  value: Text
  score: number | null
  size: number
}

export interface KeyValue {
  name: string
  kind: KeyKind
  ttlMs: number | null
  /** Fields, items, members or entries; a string's length in bytes. */
  length: number
  bytes: number | null
  encoding: string | null
  entries: Entry[]
  next: string | null
}

export interface Overview {
  version: string
  mode: string
  keys: number
  usedMemory: number
  /** `0` when no limit is set. */
  maxMemory: number
  opsPerSec: number
  keyspaceHits: number
  keyspaceMisses: number
  clients: number
  evictedKeys: number
  expiredKeys: number
}

export type KeyEdit =
  | { kind: 'setString'; value: string }
  | { kind: 'setField'; field: string; value: string }
  | { kind: 'removeField'; field: string }
  | { kind: 'setItem'; index: number; old: string; value: string }
  | { kind: 'pushItem'; value: string; head: boolean }
  | { kind: 'removeItem'; index: number; old: string }
  | { kind: 'addMember'; member: string; score: number | null }
  | { kind: 'removeMember'; member: string }
  | { kind: 'addEntry'; fields: [string, string][] }
  | { kind: 'removeEntry'; id: string }
  | { kind: 'expire'; ttlMs: number | null }
  | { kind: 'rename'; to: string }
  | { kind: 'delete' }

/** A new key and its first element (Redis has no empty hashes, lists, sets or streams). */
export interface NewKey {
  name: string
  kind: KeyKind
  field: string | null
  value: string
  score: number | null
  ttlMs: number | null
}

export const keyspaceApi = {
  overview: (connectionId: string, db: number) => invoke<Overview>('redis_overview', { connectionId, db }),
  scan: (
    connectionId: string,
    db: number,
    pattern: string,
    kind: KeyKind | null,
    cursor: string,
    limit: number,
  ) => invoke<ScanPage>('redis_scan', { connectionId, db, pattern, kind, cursor, limit }),
  readKey: (
    connectionId: string,
    db: number,
    key: string,
    cursor: string | null,
    filter: string,
    limit: number,
  ) => invoke<KeyValue>('redis_read_key', { connectionId, db, key, cursor, filter, limit }),
  /** Applies one edit while the key still has the type `kind` the browser showed. */
  editKey: (connectionId: string, db: number, key: string, kind: KeyKind, edit: KeyEdit) =>
    invoke<void>('redis_edit_key', { connectionId, db, key, kind, edit }),
  createKey: (connectionId: string, db: number, key: NewKey) =>
    invoke<void>('redis_create_key', { connectionId, db, key }),
}
