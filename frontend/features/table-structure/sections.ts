import type { TableStructure } from './api'
import { messages } from './messages'

export type Section = 'columns' | 'indexes' | 'foreignKeys' | 'triggers' | 'ddl'

export const SECTIONS: { id: Section; label: string }[] = [
  { id: 'columns', label: 'Columns' },
  { id: 'indexes', label: 'Indexes' },
  { id: 'foreignKeys', label: 'Foreign keys' },
  { id: 'triggers', label: 'Triggers' },
  { id: 'ddl', label: 'DDL' },
]

type LabelKey = 'sectionColumns' | 'sectionIndexes' | 'sectionForeignKeys' | 'sectionTriggers' | 'sectionDdl'

const LABELS: Record<Section, LabelKey> = {
  columns: 'sectionColumns',
  indexes: 'sectionIndexes',
  foreignKeys: 'sectionForeignKeys',
  triggers: 'sectionTriggers',
  ddl: 'sectionDdl',
}

/** A section's name in the language of `m`; `SECTIONS` carries the English one. */
export function sectionLabel(section: Section, m: typeof messages.en = messages.en): string {
  return m[LABELS[section]]
}

/** How many items a section lists; DDL has no count. */
export function sectionCount(structure: TableStructure, section: Section): number | null {
  return section === 'ddl' ? null : structure[section].length
}
