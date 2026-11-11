import { useState } from 'react'
import type { TableTabView } from './tabs'
import { IconButton } from '../components/Button'
import { Icon } from '../components/Icon'
import type { TableRef } from '../domain/table'
import { isRelationalDialect, type SqlDialect } from '../domain/connection'
import { DiagramView } from '../features/diagram/DiagramView'
import { ImportDialog } from '../features/import/ImportDialog'
import { NO_FILTER, type RowFilter } from '../features/table-data/rowFilter'
import { TableView } from '../features/table-data/TableView'
import { StructureSection } from '../features/table-structure/StructureSection'
import { messages as tableStructureMessages } from '../features/table-structure/messages'
import { SECTIONS, sectionCount, sectionLabel } from '../features/table-structure/sections'
import { summary } from '../features/table-structure/format'
import { useTableStructure } from '../features/table-structure/useTableStructure'
import { useMessages } from '../lib/i18n/useLanguage'
import { messages } from './messages'
import styles from './TablePage.module.css'

interface TablePageProps {
  connectionId: string
  table: TableRef
  /** The tag (e.g. PROD) that makes editing data ask first; `null` when it doesn't. */
  confirmTag: string | null
  dialect: SqlDialect
  /** Relational engines draw the table with its relations. */
  diagram: boolean
  onOpenSchemaDiagram: () => void
  /** Exports the rows the filter keeps. */
  onExport: (filter: RowFilter) => void
  /** The filter the tab opened with, e.g. from a foreign key link. */
  initialFilter?: RowFilter
  /** Opens another table of the connection, filtered. */
  onOpenTable: (table: TableRef, filter: RowFilter) => void
  /** What the tab shows; kept by the tab so the explorer can open a section. */
  view: TableTabView
  onViewChange: (view: TableTabView) => void
}

/** A table's tab: its rows, and its structure split into sections. */
export function TablePage({
  connectionId,
  table,
  confirmTag,
  dialect,
  diagram,
  onOpenSchemaDiagram,
  onExport,
  initialFilter,
  onOpenTable,
  view,
  onViewChange: setView,
}: TablePageProps) {
  const m = useMessages(messages)
  const structureM = useMessages(tableStructureMessages)
  const [filter, setFilter] = useState<RowFilter>(initialFilter ?? NO_FILTER)
  // The data view puts its row actions here, next to Export.
  const [actionsSlot, setActionsSlot] = useState<HTMLSpanElement | null>(null)
  // Loaded up front: the section counts and the size summary come from it.
  const { structure, error, loading, reload } = useTableStructure(connectionId, table)
  const [importing, setImporting] = useState(false)
  // Bumped when an import adds rows, so the data view reads its page again.
  const [revision, setRevision] = useState(0)

  return (
    <section className={styles.page}>
      <div className={styles.bar}>
        {/* The table's actions on the left, its views on the right. */}
        <span ref={setActionsSlot} className={styles.actions} />
        <IconButton label={m.exportRows} onClick={() => onExport(filter)}>
          <Icon name="download" />
        </IconButton>
        {structure?.kind !== 'view' && (
          <IconButton label={m.importRows} onClick={() => setImporting(true)}>
            <Icon name="upload" />
          </IconButton>
        )}
        {view !== 'data' && view !== 'diagram' && (
          <IconButton label={m.reloadStructure} disabled={loading} onClick={reload}>
            <Icon name="refresh" />
          </IconButton>
        )}
        <span className={styles.spacer} />
        <div role="tablist" aria-label={m.tableViews} className={styles.views}>
          <ViewTab label={m.data} selected={view === 'data'} onSelect={() => setView('data')} />
          {SECTIONS.map(({ id }) => (
            <ViewTab
              key={id}
              label={sectionLabel(id, structureM)}
              count={structure && sectionCount(structure, id)}
              selected={view === id}
              onSelect={() => setView(id)}
            />
          ))}
          {diagram && (
            <ViewTab label={m.diagram} selected={view === 'diagram'} onSelect={() => setView('diagram')} />
          )}
        </div>
      </div>

      {/* Data stays mounted so its page and scroll survive a look at the structure. */}
      <div className={styles.content} hidden={view !== 'data'}>
        <TableView
          connectionId={connectionId}
          table={table}
          confirmTag={confirmTag}
          dialect={dialect}
          filter={filter}
          onFilterChange={setFilter}
          actionsSlot={actionsSlot}
          active={view === 'data'}
          summary={structure ? summary(structure, structureM) : ''}
          foreignKeys={structure?.foreignKeys ?? []}
          columnInfo={structure?.columns}
          onOpenTable={onOpenTable}
          revision={revision}
        />
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
            <p className={styles.loading}>{m.loading}</p>
          )}
        </div>
      )}
      {importing && (
        <ImportDialog
          connectionId={connectionId}
          table={table}
          transactional={isRelationalDialect(dialect)}
          confirmTag={confirmTag}
          onClose={() => setImporting(false)}
          onImported={() => setRevision((current) => current + 1)}
        />
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
