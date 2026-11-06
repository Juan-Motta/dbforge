import { invoke } from '@tauri-apps/api/core'

export interface DiagramColumn {
  name: string
  dataType: string
  nullable: boolean
  primaryKey: boolean
}

export interface DiagramTable {
  name: string
  rowEstimate: number | null
  columns: DiagramColumn[]
}

export interface Relationship {
  name: string
  table: string
  columns: string[]
  referencedSchema: string
  referencedTable: string
  referencedColumns: string[]
}

export interface SchemaDiagram {
  tables: DiagramTable[]
  /** Foreign keys declared in the schema; some may point to tables elsewhere. */
  relationships: Relationship[]
}

export const diagramApi = {
  load: (connectionId: string, database: string, schema: string) =>
    invoke<SchemaDiagram>('schema_diagram', { connectionId, database, schema }),
}
