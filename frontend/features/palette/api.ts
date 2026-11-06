import { invoke } from '@tauri-apps/api/core'

export interface Relation {
  schema: string
  name: string
  kind: 'table' | 'view'
}

export interface RelationList {
  /** The database searched, resolved even when the connection leaves it unset. */
  database: string
  relations: Relation[]
}

export const paletteApi = {
  /** Every table and view in `database`, or in the connection's own database. */
  listRelations: (connectionId: string, database?: string) =>
    invoke<RelationList>('list_relations', { connectionId, database }),
}
