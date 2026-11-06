import { useState } from 'react'
import { IconButton } from '../components/Button'
import { Icon } from '../components/Icon'
import type { SqlDialect } from '../domain/connection'
import type { TableRef } from '../domain/table'
import { TableView } from '../features/table-data/TableView'
import { StructureSection } from '../features/table-structure/StructureSection'
import { SECTIONS, sectionCount, type Section } from '../features/table-structure/sections'
import { summary } from '../features/table-structure/format'
import { useTableStructure } from '../features/table-structure/useTableStructure'
import styles from './TablePage.module.css'

type View = 'data' | Section

interface TablePageProps {
  connectionId: string
  table: TableRef
  /** Editing a production connection's data asks for confirmation. */
  production: boolean
  dialect: SqlDialect
  onExport: () => void
}

/** A table's tab: its rows, and its structure split into sections. */
export function TablePage({ connectionId, table, production, dialect, onExport }: TablePageProps) {
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
        </div>
        <span className={styles.summary}>{structure && summary(structure)}</span>
        <IconButton label="Export rows" onClick={onExport}>
          <Icon name="download" />
        </IconButton>
        {view !== 'data' && (
          <IconButton label="Reload structure" disabled={loading} onClick={reload}>
            <Icon name="refresh" />
          </IconButton>
        )}
      </div>

      {/* Data stays mounted so its page and scroll survive a look at the structure. */}
      <div className={styles.content} hidden={view !== 'data'}>
        <TableView connectionId={connectionId} table={table} production={production} />
      </div>
      {view !== 'data' && (
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
