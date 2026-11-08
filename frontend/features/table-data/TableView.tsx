import { useState, type KeyboardEvent } from 'react'
import { createPortal } from 'react-dom'
import { Badge } from '../../components/Badge'
import { Button, IconButton } from '../../components/Button'
import { SqlCode } from '../../components/code/SqlCode'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import type { SqlDialect } from '../../domain/connection'
import type { TableRef } from '../../domain/table'
import type { ForeignKey } from '../table-structure/api'
import { linkFilter, referencedTable } from './foreignLinks'
import { errorMessage } from '../../lib/errors'
import { tableDataApi, type ApplySummary } from './api'
import { EditableGrid } from './EditableGrid'
import {
  addRow,
  counts,
  editCell,
  editNewCell,
  hasEdits,
  keyOf,
  NO_EDITS,
  removeNewRow,
  rowKey,
  toChanges,
  toggleDelete,
  unsetNewCell,
  type Edits,
} from './edits'
import { FilterBar } from './FilterBar'
import { isFiltered, sortBy, sortOf, type RowFilter } from './rowFilter'
import { DEFAULT_PAGE_SIZE, PAGE_SIZES, useRowsPage } from './useRowsPage'
import styles from './TableView.module.css'

interface TableViewProps {
  connectionId: string
  table: TableRef
  /** Applying changes on a production connection asks for confirmation first. */
  production: boolean
  dialect: SqlDialect
  /** What the rows are narrowed and ordered by; the page owns it so exports use it too. */
  filter: RowFilter
  onFilterChange: (filter: RowFilter) => void
  /** Where the row actions go: the table page's top bar. */
  actionsSlot: HTMLElement | null
  /** The data view is the one shown, so its actions belong in the top bar. */
  active: boolean
  /** Size and row estimate, for the footer. */
  summary: string
  /** The table's foreign keys: their columns link to the rows they point to. */
  foreignKeys: ForeignKey[]
  /** Opens a table filtered to the rows a foreign key points to. */
  onOpenTable: (table: TableRef, filter: RowFilter) => void
}

type Status =
  | { state: 'idle' }
  | { state: 'applying' }
  | { state: 'applied'; summary: ApplySummary }
  | { state: 'failed'; error: string }

/** A table's rows, one page at a time, editable when the table has a primary key. */
export function TableView({
  connectionId,
  table,
  production,
  dialect,
  filter,
  onFilterChange,
  actionsSlot,
  active,
  summary,
  foreignKeys,
  onOpenTable,
}: TableViewProps) {
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE)
  const { page, error, loading, offset, hasPrevious, hasNext, previous, next, reload } = useRowsPage(
    connectionId,
    table,
    filter,
    pageSize,
  )
  const [edits, setEdits] = useState<Edits>(NO_EDITS)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [status, setStatus] = useState<Status>({ state: 'idle' })
  const [preview, setPreview] = useState<string[] | null>(null)
  const [confirming, setConfirming] = useState(false)

  const columns = page?.columns ?? []
  const rows = page?.rows ?? []
  const editable = columns.some((c) => c.primaryKey)
  const pending = hasEdits(edits)
  const applying = status.state === 'applying'
  const { updated, inserted, deleted } = counts(edits)
  const total = updated + inserted + deleted

  const change = (next: (current: Edits) => Edits) => {
    setEdits(next)
    if (status.state !== 'idle') setStatus({ state: 'idle' })
  }

  const select = (id: string, additive: boolean) =>
    setSelected((current) => {
      if (!additive) return current.size === 1 && current.has(id) ? new Set() : new Set([id])
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  /** Marks the selected rows for deletion (or unmarks them); new rows are simply removed. */
  const deleteSelected = () => {
    change((current) =>
      [...selected].reduce((acc, id) => {
        if (id.startsWith('new:')) return removeNewRow(acc, Number(id.slice(4)))
        const row = rows.find((r) => rowKey(keyOf(r, columns)) === id)
        return row ? toggleDelete(acc, keyOf(row, columns)) : acc
      }, current),
    )
    setSelected(new Set())
  }

  const apply = async () => {
    setConfirming(false)
    setStatus({ state: 'applying' })
    try {
      const summary = await tableDataApi.applyChanges(connectionId, table, toChanges(edits))
      setEdits(NO_EDITS)
      setSelected(new Set())
      setStatus({ state: 'applied', summary })
      reload()
    } catch (failure) {
      setStatus({ state: 'failed', error: errorMessage(failure) })
    }
  }

  const requestApply = () => {
    if (!pending || applying) return
    if (production) setConfirming(true)
    else void apply()
  }

  const showPreview = async () => {
    try {
      setPreview(await tableDataApi.previewChanges(connectionId, table, toChanges(edits)))
    } catch (failure) {
      setStatus({ state: 'failed', error: errorMessage(failure) })
    }
  }

  const discard = () => {
    setEdits(NO_EDITS)
    setSelected(new Set())
    setStatus({ state: 'idle' })
  }

  const onKeyDown = (event: KeyboardEvent) => {
    if ((event.metaKey || event.ctrlKey) && event.key === 's') {
      event.preventDefault()
      requestApply()
    }
  }

  const rowCount = rows.length
  // Moving to another page would hide pending rows; apply or discard first.
  const lockPaging = loading || pending
  const noKey = 'This table has no primary key, so its rows cannot be identified safely to edit them'

  return (
    <section className={styles.view} aria-label={`${table.schema}.${table.name}`} onKeyDown={onKeyDown}>
      <FilterBar dialect={dialect} value={filter} onApply={onFilterChange} disabled={pending} />
      {active &&
        actionsSlot &&
        createPortal(
          <>
            <IconButton
              label={editable ? 'Add row' : noKey}
              disabled={!editable || applying}
              onClick={() => change(addRow)}
            >
              <Icon name="plus" />
            </IconButton>
            <IconButton
              label={editable ? 'Delete selected rows' : noKey}
              disabled={!editable || selected.size === 0 || applying}
              onClick={deleteSelected}
            >
              <Icon name="trash" />
            </IconButton>
            <IconButton label="Reload rows" disabled={lockPaging} onClick={reload}>
              <Icon name="refresh" />
            </IconButton>
          </>,
          actionsSlot,
        )}

      <div className={styles.content} aria-busy={loading}>
        {error ? (
          <p className={styles.error}>{error}</p>
        ) : (
          page && (
            <EditableGrid
              columns={columns}
              rows={rows}
              firstRowNumber={offset + 1}
              edits={edits}
              editable={editable && !applying}
              selected={selected}
              onSelect={select}
              onEditCell={(r, c, value) =>
                change((current) =>
                  editCell(current, keyOf(rows[r], columns), columns[c].name, value, rows[r][c]),
                )
              }
              onUnsetNewCell={(id, c) => change((current) => unsetNewCell(current, id, columns[c].name))}
              onEditNewCell={(id, c, value) =>
                change((current) => editNewCell(current, id, columns[c].name, value))
              }
              linkOf={(r, c) => {
                const key = foreignKeys.find((k) => k.columns.includes(columns[c].name))
                if (!key) return null
                const values = Object.fromEntries(columns.map((column, i) => [column.name, rows[r][i]]))
                const target = linkFilter(dialect, key, values)
                return target && (() => onOpenTable(referencedTable(table, key.referencedTable), target))
              }}
              sortOf={(column) => sortOf(dialect, column, filter.order)}
              onSort={pending ? undefined : (column) => onFilterChange(sortBy(dialect, column, filter))}
            />
          )
        )}
      </div>

      {/* Rows and paging, and in their place while editing, the pending changes. */}
      <div className={styles.footer} data-state={pending ? 'pending' : status.state}>
        {pending && (
          <>
            <Badge tone="warn">
              {total} pending {total === 1 ? 'change' : 'changes'}
            </Badge>
            <span className={styles.detail}>
              {updated} updated · {inserted} inserted · {deleted} deleted
            </span>
            <Button variant="ghost" disabled={applying} onClick={showPreview}>
              Preview SQL
            </Button>
          </>
        )}
        {!pending && status.state === 'applied' && (
          <span className={styles.applied}>
            <Icon name="check" /> Applied: {status.summary.updated} updated · {status.summary.inserted}{' '}
            inserted · {status.summary.deleted} deleted
          </span>
        )}
        {status.state === 'failed' && (
          <span className={styles.failed} title={status.error}>
            {status.error}
          </span>
        )}
        {!pending && status.state === 'idle' && <span className={styles.summary}>{summary}</span>}
        <span className={styles.spacer} />
        <span className={styles.range} aria-live="polite">
          {loading
            ? 'Loading…'
            : rowCount === 0
              ? isFiltered(filter)
                ? 'No rows match'
                : 'No rows'
              : `Rows ${offset + 1}–${offset + rowCount}${isFiltered(filter) ? ' · filtered' : ''}`}
        </span>
        <label className={styles.pageSize}>
          <select
            aria-label="Rows per page"
            value={pageSize}
            disabled={lockPaging}
            onChange={(event) => setPageSize(Number(event.target.value))}
          >
            {PAGE_SIZES.map((size) => (
              <option key={size} value={size}>
                {size} per page
              </option>
            ))}
          </select>
        </label>
        <IconButton label="Previous page" disabled={!hasPrevious || lockPaging} onClick={previous}>
          <Icon name="chevronLeft" />
        </IconButton>
        <IconButton label="Next page" disabled={!hasNext || lockPaging} onClick={next}>
          <Icon name="chevronRight" />
        </IconButton>
        {pending && (
          <>
            <span className={styles.divider} />
            <Button disabled={applying} onClick={discard}>
              Discard
            </Button>
            <Button variant="primary" disabled={applying} onClick={requestApply}>
              {applying ? 'Applying…' : 'Apply'}
              <span className={styles.shortcut}>⌘S</span>
            </Button>
          </>
        )}
      </div>

      {preview && (
        <Modal
          title="Pending changes as SQL"
          onClose={() => setPreview(null)}
          className={styles.previewDialog}
        >
          <p className={styles.previewNote}>
            Runs in one transaction. If any row changed since it was loaded, none of it applies.
          </p>
          <div className={styles.previewCode}>
            <SqlCode value={preview.map((statement) => `${statement};`).join('\n')} />
          </div>
        </Modal>
      )}

      {confirming && (
        <Modal title="Apply changes to production?" onClose={() => setConfirming(false)}>
          <div className={styles.confirm}>
            <p>
              This connection is marked <strong>Production</strong>. You are about to change{' '}
              <code>
                {table.schema}.{table.name}
              </code>
              : {updated} updated, {inserted} inserted, {deleted} deleted.
            </p>
            <div className={styles.confirmActions}>
              <Button data-autofocus onClick={() => setConfirming(false)}>
                Cancel
              </Button>
              <Button variant="primary" onClick={() => void apply()}>
                Apply to production
              </Button>
            </div>
          </div>
        </Modal>
      )}
    </section>
  )
}
