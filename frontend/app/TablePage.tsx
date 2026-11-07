import { useState } from 'react'
import { IconButton } from '../components/Button'
import { Icon } from '../components/Icon'
import type { SqlDialect } from '../domain/connection'
import type { TableRef } from '../domain/table'
import { DiagramView } from '../features/diagram/DiagramView'
import { TableView } from '../features/table-data/TableView'
import { StructureSection } from '../features/table-structure/StructureSection'
import { SECTIONS, sectionCount, type Section } from '../features/table-structure/sections'
import { summary } from '../features/table-structure/format'
import { useTableStructure } from '../features/table-structure/useTableStructure'
import styles from './TablePage.module.css'

type View = 'data' | 'diagram' | Section

interface TablePageProps {
  connectionId: string
  table: TableRef
  /** Editing a production connection's data asks for confirmation. */
  production: boolean
  dialect: SqlDialect
  /** Relational engines draw the table with its relations. */
  diagram: boolean
  onOpenSchemaDiagram: () => void
  onExport: () => void
}

/** A table's tab: its rows, and its structure split into sections. */
export function TablePage({
  connectionId,
  table,
  production,
  dialect,
  diagram,
  onOpenSchemaDiagram,
  onExport,
}: TablePageProps) {
  const [view, setView] = useState<View>('data')
  // Loaded up front: the section counts and the size summary come from it.
  const { structure, error, loading, reload } = useTableStructure(connectionId, table)

  return (
    <section className={styles.page}>
      <div className={styles.bar}>
        <div role="tablist" aria-label="Table views" className={styles.views}>
          <ViewTab label="Data" selected={view === 'data'} onSelect={() => setView('data')} />
          {SECTIONS.map(({ id, label }) => (
            <ViewTab
              key={id}
              label={label}
              count={structure && sectionCount(structure, id)}
              selected={view === id}
              onSelect={() => setView(id)}
            />
          ))}
          {diagram && (
            <ViewTab label="Diagram" selected={view === 'diagram'} onSelect={() => setView('diagram')} />
          )}
        </div>
        <span className={styles.summary}>{structure && summary(structure)}</span>
        <IconButton label="Export rows" onClick={onExport}>
          <Icon name="download" />
        </IconButton>
        {view !== 'data' && view !== 'diagram' && (
          <IconButton label="Reload structure" disabled={loading} onClick={reload}>
            <Icon name="refresh" />
          </IconButton>
        )}
      </div>

      {/* Data stays mounted so its page and scroll survive a look at the structure. */}
      <div className={styles.content} hidden={view !== 'data'}>
        <TableView connectionId={connectionId} table={table} production={production} />
      </div>
      {view === 'diagram' && (
        <div className={styles.content}>
          <DiagramView
            connectionId={connectionId}
            database={table.database}
            schema={table.schema}
            focus={table.name}
            onOpenSchema={onOpenSchemaDiagram}
          />
        </div>
      )}
      {view !== 'data' && view !== 'diagram' && (
        <div className={styles.content} aria-busy={loading}>
          {error ? (
            <p className={styles.error}>{error}</p>
          ) : structure ? (
            <StructureSection structure={structure} section={view} dialect={dialect} />
          ) : (
            <p className={styles.loading}>Loading…</p>
          )}
        </div>
      )}
    </section>
  )
}

interface ViewTabProps {
  label: string
  count?: number | null
  selected: boolean
  onSelect: () => void
}

function ViewTab({ label, count, selected, onSelect }: ViewTabProps) {
  return (
    <button type="button" role="tab" aria-selected={selected} className={styles.view} onClick={onSelect}>
      {label}
      {count != null && <span className={styles.count}>{count}</span>}
    </button>
  )
}
