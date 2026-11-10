import { invoke } from '@tauri-apps/api/core'

/** `string`, `hash`, `list`, `set`, `zset`, `stream`, or a module's type. */
export type KeyKind = string

export interface KeyInfo {
  /** The name as shown: text, or hex when `binary` says its bytes aren't text. */
  name: string
  binary: boolean
  kind: KeyKind
  /** Milliseconds left; `null` when the key never expires. */
  ttlMs: number | null
}

export interface ScanPage {
  keys: KeyInfo[]
  /** Where the next page starts; `"0"` when the scan is complete. */
  cursor: string
}

/**
 * Bytes for display: the text when it is UTF-8, else hex. Sent back as it
 * came to name a key, field, item or member, so one that isn't text is found.
 */
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
  binary: boolean
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
  | { kind: 'setField'; field: Text; value: string }
  | { kind: 'removeField'; field: Text }
  | { kind: 'setItem'; index: number; old: Text; value: string }
  | { kind: 'pushItem'; value: string; head: boolean }
  | { kind: 'removeItem'; index: number; old: Text }
  | { kind: 'addMember'; member: Text; score: number | null }
  | { kind: 'removeMember'; member: Text }
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

/** A name typed here, which is text. */
export const typed = (text: string): Text => ({ text, binary: false })

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
    key: Text,
    cursor: string | null,
    filter: string,
    limit: number,
  ) => invoke<KeyValue>('redis_read_key', { connectionId, db, key, cursor, filter, limit }),
  /** Applies one edit while the key still has the type `kind` the browser showed. */
  editKey: (connectionId: string, db: number, key: Text, kind: KeyKind, edit: KeyEdit) =>
    invoke<void>('redis_edit_key', { connectionId, db, key, kind, edit }),
  createKey: (connectionId: string, db: number, key: NewKey) =>
    invoke<void>('redis_create_key', { connectionId, db, key }),
}
