import type { TableStructure } from './api'

export type Section = 'columns' | 'indexes' | 'foreignKeys' | 'triggers' | 'ddl'

export const SECTIONS: { id: Section; label: string }[] = [
  { id: 'columns', label: 'Columns' },
  { id: 'indexes', label: 'Indexes' },
  { id: 'foreignKeys', label: 'Foreign keys' },
  { id: 'triggers', label: 'Triggers' },
  { id: 'ddl', label: 'DDL' },
]

/** How many items a section lists; DDL has no count. */
export function sectionCount(structure: TableStructure, section: Section): number | null {
  return section === 'ddl' ? null : structure[section].length
}
