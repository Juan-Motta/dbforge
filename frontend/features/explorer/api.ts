import { invoke } from '@tauri-apps/api/core'

/** One object returned by the backend for a level of the tree. */
export interface DbObject {
  name: string
  kind: 'database' | 'schema' | 'table' | 'view'
  rowEstimate: number | null
}

export const explorerApi = {
  /** `[]` → databases, `[db]` → schemas, `[db, schema]` → tables and views. */
  listObjects: (connectionId: string, path: string[]) =>
    invoke<DbObject[]>('list_objects', { connectionId, path }),
}
