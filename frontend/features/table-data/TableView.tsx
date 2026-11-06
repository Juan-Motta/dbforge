import { IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import type { TableRef } from '../../domain/table'
import { DataGrid } from '../../components/data-grid/DataGrid'
import { useRowsPage } from './useRowsPage'
import styles from './TableView.module.css'

interface TableViewProps {
  connectionId: string
  table: TableRef
}

/** Read-only view of a table's rows, one page at a time. */
export function TableView({ connectionId, table }: TableViewProps) {
  const { page, error, loading, offset, hasPrevious, hasNext, previous, next, reload } = useRowsPage(
    connectionId,
    table,
  )
  const rowCount = page?.rows.length ?? 0

  return (
    <section className={styles.view} aria-label={`${table.schema}.${table.name}`}>
      <div className={styles.toolbar}>
        <Icon name="table" />
        <span className={styles.title}>
          <span className="muted">{table.schema}.</span>
          {table.name}
        </span>
        <span className={styles.spacer} />
        <span className={styles.range} aria-live="polite">
          {loading ? 'Loading…' : rowCount === 0 ? 'No rows' : `Rows ${offset + 1}–${offset + rowCount}`}
        </span>
        <IconButton label="Previous page" disabled={!hasPrevious || loading} onClick={previous}>
          <Icon name="chevronLeft" />
        </IconButton>
        <IconButton label="Next page" disabled={!hasNext || loading} onClick={next}>
          <Icon name="chevronRight" />
        </IconButton>
        <IconButton label="Reload" disabled={loading} onClick={reload}>
          <Icon name="refresh" />
        </IconButton>
      </div>

      <div className={styles.content} aria-busy={loading}>
        {error ? (
          <p className={styles.error}>{error}</p>
        ) : (
          page && <DataGrid columns={page.columns} rows={page.rows} firstRowNumber={offset + 1} />
        )}
      </div>
    </section>
  )
}
