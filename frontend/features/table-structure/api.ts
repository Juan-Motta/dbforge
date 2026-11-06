import { invoke } from '@tauri-apps/api/core'
import type { TableRef } from '../../domain/table'

export interface ColumnInfo {
  name: string
  dataType: string
  nullable: boolean
  /** The default expression, or how the value is generated (identity, computed). */
  default: string | null
  comment: string | null
}

export interface IndexInfo {
  name: string
  /** Access method, e.g. `btree` or `gin`. */
  method: string
  unique: boolean
  primary: boolean
  /** Key columns or expressions, in order. */
  columns: string[]
  bytes: number
}

export interface ForeignKey {
  name: string
  columns: string[]
  referencedTable: string
  referencedColumns: string[]
  /** SQL action names, e.g. `NO ACTION` or `CASCADE`. */
  onUpdate: string
  onDelete: string
}

export interface TriggerInfo {
  name: string
  enabled: boolean
  /** The full `CREATE TRIGGER` statement. */
  definition: string
}

export interface TableStructure {
  kind: 'table' | 'view'
  rowEstimate: number | null
  dataBytes: number
  indexBytes: number
  comment: string | null
  columns: ColumnInfo[]
  indexes: IndexInfo[]
  foreignKeys: ForeignKey[]
  triggers: TriggerInfo[]
  /** A script that rebuilds the object, comments and triggers included. */
  ddl: string
}

export const structureApi = {
  get: (connectionId: string, table: TableRef) =>
    invoke<TableStructure>('table_structure', { connectionId, table }),
}
