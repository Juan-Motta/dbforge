import { invoke } from '@tauri-apps/api/core'
import type { NewObject } from './newObject'

export const adminApi = {
  /** Creates a database, role or user on the connection's server. */
  create: (connectionId: string, object: NewObject) =>
    invoke<void>('create_object', { request: { connectionId, object } }),
  /** Drops a database, and everything in it, from the connection's server. */
  dropDatabase: (connectionId: string, database: string) =>
    invoke<void>('drop_database', { request: { connectionId, database } }),
}
