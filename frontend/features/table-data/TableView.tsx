import { useState, type KeyboardEvent } from 'react'
import { createPortal } from 'react-dom'
import { Badge } from '../../components/Badge'
import { Button, IconButton } from '../../components/Button'
import { SqlCode } from '../../components/code/SqlCode'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import type { SqlDialect } from '../../domain/connection'
import type { TableRef } from '../../domain/table'
import type { ColumnInfo, ForeignKey } from '../table-structure/api'
import { linkFilter, referencedTable } from './foreignLinks'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import { tableDataApi, type ApplySummary } from './api'
import { EditableGrid } from './EditableGrid'
import {
  addDocument,
  addRow,
  cellAt,
  counts,
  addRowWith,
  DEFAULT,
  editCell,
  editNewCell,
  editNewDocument,
  hasEdits,
  isDefault,
  keyOf,
  NO_EDITS,
  removeNewDocument,
  removeNewRow,
  replaceDocument,
  rowKey,
  toChanges,
  toggleDelete,
  unsetNewCell,
  type Cell,
  type CellValue,
  type Edits,
  type RowChange,
} from './edits'
import { DocumentList, NEW_DOCUMENT, type LoadedDocument } from './DocumentList'
import { CellInspector, type InspectedCell } from './CellInspector'
import { FilterBar } from './FilterBar'
import { narrowedBy, sortedBy } from './cellActions'
import { NO_SELECTION, selectRows, type RowSelection, type SelectMode } from './rowSelection'
import { isFiltered, sortBy, sortOf, type RowFilter } from './rowFilter'
import { DEFAULT_PAGE_SIZE, PAGE_SIZES, useRowsPage } from './useRowsPage'
import styles from './TableView.module.css'

interface TableViewProps {
  connectionId: string
  table: TableRef
  /** The tag (e.g. PROD) that makes applying changes ask first; `null` when it doesn't. */
  confirmTag: string | null
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
  /** The table's columns as its structure describes them, once loaded: which take NULL or have a DEFAULT. */
  columnInfo?: ColumnInfo[]
  /** Opens a table filtered to the rows a foreign key points to. */
  onOpenTable: (table: TableRef, filter: RowFilter) => void
}

/** How the changes run: one transaction that applies all or nothing, except where noted. */
function previewNote(dialect: SqlDialect, m: typeof messages.en): string {
  if (dialect === 'mongodb') return m.previewNoteMongodb
  if (dialect === 'cosmosdb') return m.previewNoteCosmosdb
  return m.previewNote
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
  confirmTag,
  dialect,
  filter,
  onFilterChange,
  actionsSlot,
  active,
  summary,
  foreignKeys,
  columnInfo,
  onOpenTable,
}: TableViewProps) {
  const m = useMessages(messages)
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE)
  const { page, error, loading, offset, hasPrevious, hasNext, previous, next, reload } = useRowsPage(
    connectionId,
    table,
    filter,
    pageSize,
  )
  const [edits, setEdits] = useState<Edits>(NO_EDITS)
  const [selection, setSelection] = useState<RowSelection>(NO_SELECTION)
  const selected = selection.selected
  const [status, setStatus] = useState<Status>({ state: 'idle' })
  const [preview, setPreview] = useState<string[] | null>(null)
  const [confirming, setConfirming] = useState(false)
  // The cell a click picked, and whether the side panel shows its value.
  const [picked, setPicked] = useState<Cell | null>(null)
  const [inspecting, setInspecting] = useState(false)
  // Document databases show their rows as a grid or as whole documents.
  const [asDocuments, setAsDocuments] = useState(false)
  // The document open in an editor: a loaded row's key, or `doc:<n>` for a new one.
  const [editingDocument, setEditingDocument] = useState<string | null>(null)
  // Another page, order or table: the picked cell and the open document are gone.
  const [shownRows, setShownRows] = useState(page?.rows)
  if (shownRows !== page?.rows) {
    setShownRows(page?.rows)
    setPicked(null)
    setEditingDocument(null)
  }

  const columns = page?.columns ?? []
  const rows = page?.rows ?? []
  const documents = page?.documents
  const showingDocuments = asDocuments && documents !== undefined
  const editable = columns.some((c) => c.primaryKey)
  const pending = hasEdits(edits)
  const applying = status.state === 'applying'
  const { updated, inserted, deleted } = counts(edits)
  const total = updated + inserted + deleted

  const change = (next: (current: Edits) => Edits) => {
    setEdits(next)
    if (status.state !== 'idle') setStatus({ state: 'idle' })
  }

  /** The rows as the grid shows them, new ones first: the order a Shift-click or a drag selects in. */
  const rowOrder = [
    ...edits.inserts.map((row) => `new:${row.id}`),
    ...(page?.rows ?? []).map((row) => rowKey(keyOf(row, page?.columns ?? []))),
  ]
  const select = (id: string, mode: SelectMode) =>
    setSelection((current) => selectRows(current, id, mode, rowOrder))

  /** A change to row `id` reaches every selected row when `id` is one of several selected. */
  const targetsOf = (id: string) => (selected.size > 1 && selected.has(id) ? [...selected] : [id])

  /** Puts `value` in column `c` of each row in `ids`; rows marked for deletion keep theirs. */
  const editRows = (current: Edits, ids: string[], c: number, value: CellValue | null): Edits =>
    ids.reduce((acc, id) => {
      const name = columns[c].name
      if (id.startsWith('new:')) {
        const newId = Number(id.slice(4))
        return isDefault(value) ? unsetNewCell(acc, newId, name) : editNewCell(acc, newId, name, value)
      }
      if (acc.deletes.has(id)) return acc
      const row = rows.find((r) => rowKey(keyOf(r, columns)) === id)
      return row ? editCell(acc, keyOf(row, columns), name, value, row[c]) : acc
    }, current)

  /** Marks the selected rows for deletion (or unmarks them); new rows are simply removed. */
  const deleteSelected = () => {
    change((current) =>
      [...selected].reduce((acc, id) => {
        if (id.startsWith('new:')) return removeNewRow(acc, Number(id.slice(4)))
        if (id.startsWith('doc:')) return removeNewDocument(acc, Number(id.slice(4)))
        const row = rows.find((r) => rowKey(keyOf(r, columns)) === id)
        return row ? toggleDelete(acc, keyOf(row, columns)) : acc
      }, current),
    )
    setSelection(NO_SELECTION)
  }

  const apply = async () => {
    setConfirming(false)
    setStatus({ state: 'applying' })
    try {
      const summary = await tableDataApi.applyChanges(connectionId, table, toChanges(edits))
      setEdits(NO_EDITS)
      setSelection(NO_SELECTION)
      setEditingDocument(null)
      setStatus({ state: 'applied', summary })
      reload()
    } catch (failure) {
      setStatus({ state: 'failed', error: errorMessage(failure) })
    }
  }

  const requestApply = () => {
    if (!pending || applying) return
    if (confirmTag) setConfirming(true)
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
    setSelection(NO_SELECTION)
    setEditingDocument(null)
    setStatus({ state: 'idle' })
  }

  const onKeyDown = (event: KeyboardEvent) => {
    if ((event.metaKey || event.ctrlKey) && event.key === 's') {
      event.preventDefault()
      requestApply()
    }
  }

  const noKey = m.noKey
  const inspected = ((): InspectedCell | null => {
    const cell = picked && cellAt(edits, rows, columns, picked)
    if (!picked || !cell) return null
    const column = columns[picked.column]
    const info = columnInfo?.find((c) => c.name === column.name)
    return {
      column,
      value: cell.value,
      unset: cell.unset,
      // Until the structure loads, NULL is offered; the server has the last word.
      nullable: info?.nullable ?? true,
      options: info?.options,
      locked: !editable
        ? noKey
        : applying
          ? m.applyingChanges
          : column.generated
            ? m.computed
            : cell.deleted
              ? m.rowDeleted
              : null,
    }
  })()

  /** The inspector's value, as a pending edit of the picked cell. */
  const saveInspected = (value: string | null) => {
    if (!picked) return
    const id = picked.row.startsWith('new:')
      ? picked.row
      : rowKey(keyOf(rows[Number(picked.row.slice(4))], columns))
    change((current) => editRows(current, targetsOf(id), picked.column, value))
  }

  /** A new document, open in its editor. */
  const newDocument = () => {
    const next = addDocument(edits, NEW_DOCUMENT)
    change(() => next)
    setEditingDocument(`doc:${next.newDocuments[0].id}`)
  }

  /** What the database would say about a change, without making it: `null` when it can be made. */
  const check = async (one: RowChange) => {
    try {
      await tableDataApi.previewChanges(connectionId, table, [one])
      return null
    } catch (failure) {
      return errorMessage(failure)
    }
  }

  const loadedDocuments: LoadedDocument[] = (documents ?? []).map((text, i) => {
    const key = keyOf(rows[i], columns)
    return { id: rowKey(key), key, text }
  })

  const rowCount = rows.length
  // Moving to another page would hide pending rows; apply or discard first.
  const lockPaging = loading || pending

  return (
    <section className={styles.view} aria-label={`${table.schema}.${table.name}`} onKeyDown={onKeyDown}>
      <FilterBar dialect={dialect} value={filter} onApply={onFilterChange} disabled={pending} />
      <div className={styles.body}>
        {active &&
          actionsSlot &&
          createPortal(
            <>
              {documents !== undefined && (
                <IconButton
                  label={pending ? m.switchViewsLocked : showingDocuments ? m.showAsTable : m.showAsDocuments}
                  active={showingDocuments}
                  disabled={pending}
                  onClick={() => setAsDocuments(!asDocuments)}
                >
                  <Icon name="code" />
                </IconButton>
              )}
              <IconButton
                label={editable ? (showingDocuments ? m.addDocument : m.addRow) : noKey}
                disabled={!editable || applying}
                onClick={showingDocuments ? newDocument : () => change(addRow)}
              >
                <Icon name="plus" />
              </IconButton>
              <IconButton
                label={editable ? m.deleteSelected : noKey}
                disabled={!editable || selected.size === 0 || applying}
                onClick={deleteSelected}
              >
                <Icon name="trash" />
              </IconButton>
              <IconButton label={m.reloadRows} disabled={lockPaging} onClick={reload}>
                <Icon name="refresh" />
              </IconButton>
              <IconButton
                label={showingDocuments ? m.inspectInTable : m.inspectCell}
                active={inspecting && !showingDocuments}
                disabled={showingDocuments}
                onClick={() => setInspecting(!inspecting)}
              >
                <Icon name="eye" />
              </IconButton>
            </>,
            actionsSlot,
          )}

        <div className={styles.content} aria-busy={loading}>
          {error ? (
            <p className={styles.error}>{error}</p>
          ) : showingDocuments ? (
            <DocumentList
              documents={loadedDocuments}
              edits={edits}
              language={dialect === 'mongodb' ? 'javascript' : 'json'}
              firstNumber={offset + 1}
              locked={!editable ? noKey : applying ? m.applyingChanges : null}
              selected={selected}
              onSelect={(id, additive) => select(id, additive ? 'toggle' : 'single')}
              editing={editingDocument}
              onEdit={setEditingDocument}
              check={check}
              onReplace={(key, text, original) =>
                change((current) => replaceDocument(current, key, text, original))
              }
              onEditNew={(id, text) => change((current) => editNewDocument(current, id, text))}
              onRemoveNew={(id) => change((current) => removeNewDocument(current, id))}
            />
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
                  change((current) => editRows(current, targetsOf(rowKey(keyOf(rows[r], columns))), c, value))
                }
                onUnsetNewCell={(id, c) =>
                  change((current) => editRows(current, targetsOf(`new:${id}`), c, DEFAULT))
                }
                onEditNewCell={(id, c, value) =>
                  change((current) => editRows(current, targetsOf(`new:${id}`), c, value))
                }
                linkOf={(r, c) => {
                  const key = foreignKeys.find((k) => k.columns.includes(columns[c].name))
                  if (!key) return null
                  const values = Object.fromEntries(columns.map((column, i) => [column.name, rows[r][i]]))
                  const target = linkFilter(dialect, key, values)
                  return target && (() => onOpenTable(referencedTable(table, key.referencedTable), target))
                }}
                columnInfo={columnInfo && Object.fromEntries(columnInfo.map((c) => [c.name, c]))}
                active={picked}
                onActivate={setPicked}
                onInspect={() => setInspecting(!inspecting)}
                sortOf={(column) => sortOf(dialect, column, filter.order)}
                onSort={pending ? undefined : (column) => onFilterChange(sortBy(dialect, column, filter))}
                dialect={dialect}
                table={table}
                onNarrow={pending ? undefined : (condition) => onFilterChange(narrowedBy(filter, condition))}
                onSortColumn={
                  pending
                    ? undefined
                    : (column, direction) => onFilterChange(sortedBy(dialect, column, direction, filter))
                }
                onDuplicateRow={(r) =>
                  change((current) =>
                    addRowWith(
                      current,
                      Object.fromEntries(
                        columns
                          // The copy gets its own key and lets the database compute what it computes.
                          .map((column, c) => [column, rows[r][c]] as const)
                          .filter(([column]) => !column.primaryKey && !column.generated)
                          .map(([column, value]) => [column.name, value]),
                      ),
                    ),
                  )
                }
                onDeleteRow={(r) => change((current) => toggleDelete(current, keyOf(rows[r], columns)))}
              />
            )
          )}
        </div>
        {inspecting && !showingDocuments && (
          <CellInspector
            // Another cell, or a value just saved or reverted, starts a fresh draft.
            key={picked ? `${picked.row}:${picked.column}:${inspected?.value}` : 'none'}
            cell={inspected}
            onSave={saveInspected}
            onClose={() => setInspecting(false)}
          />
        )}
      </div>

      {/* Rows and paging, and in their place while editing, the pending changes. */}
      <div className={styles.footer} data-state={pending ? 'pending' : status.state}>
        {pending && (
          <>
            <Badge tone="warn">{m.pendingChanges(total)}</Badge>
            <span className={styles.detail}>{m.changeCounts(updated, inserted, deleted)}</span>
            <Button variant="ghost" disabled={applying} onClick={showPreview}>
              {m.previewSql}
            </Button>
          </>
        )}
        {!pending && status.state === 'applied' && (
          <span className={styles.applied}>
            <Icon name="check" />{' '}
            {m.applied(status.summary.updated, status.summary.inserted, status.summary.deleted)}
          </span>
        )}
        {status.state === 'failed' && (
          <span className={styles.failed} title={status.error}>
            {status.error}
          </span>
        )}
        {!pending && status.state === 'idle' && <span className={styles.summary}>{summary}</span>}
        {selected.size > 1 && <Badge tone="info">{m.selectionHint(selected.size)}</Badge>}
        <span className={styles.spacer} />
        <span className={styles.range} aria-live="polite">
          {loading
            ? m.loading
            : rowCount === 0
              ? isFiltered(filter)
                ? m.noRowsMatch
                : m.noRows
              : m.rowsRange(offset + 1, offset + rowCount, isFiltered(filter))}
        </span>
        <label className={styles.pageSize}>
          <select
            aria-label={m.rowsPerPage}
            value={pageSize}
            disabled={lockPaging}
            onChange={(event) => setPageSize(Number(event.target.value))}
          >
            {PAGE_SIZES.map((size) => (
              <option key={size} value={size}>
                {m.perPage(size)}
              </option>
            ))}
          </select>
        </label>
        <IconButton label={m.previousPage} disabled={!hasPrevious || lockPaging} onClick={previous}>
          <Icon name="chevronLeft" />
        </IconButton>
        <IconButton label={m.nextPage} disabled={!hasNext || lockPaging} onClick={next}>
          <Icon name="chevronRight" />
        </IconButton>
        {pending && (
          <>
            <span className={styles.divider} />
            <Button disabled={applying} onClick={discard}>
              {m.discard}
            </Button>
            <Button variant="primary" disabled={applying} onClick={requestApply}>
              {applying ? m.applying : m.apply}
              <span className={styles.shortcut}>⌘S</span>
            </Button>
          </>
        )}
      </div>

      {preview && (
        <Modal title={m.previewTitle} onClose={() => setPreview(null)} className={styles.previewDialog}>
          <p className={styles.previewNote}>{previewNote(dialect, m)}</p>
          <div className={styles.previewCode}>
            <SqlCode
              value={preview.map((line) => (line.endsWith(';') ? line : `${line};`)).join('\n')}
              dialect={dialect}
            />
          </div>
        </Modal>
      )}

      {confirming && (
        <Modal title={m.confirmTitle(confirmTag ?? '')} onClose={() => setConfirming(false)}>
          <div className={styles.confirm}>
            <p>
              {m.confirmMarked} <strong>{confirmTag}</strong>. {m.confirmChange}{' '}
              <code>
                {table.schema}.{table.name}
              </code>
              : {m.confirmCounts(updated, inserted, deleted)}.
            </p>
            <div className={styles.confirmActions}>
              <Button data-autofocus onClick={() => setConfirming(false)}>
                {m.cancel}
              </Button>
              <Button variant="primary" onClick={() => void apply()}>
                {m.applyOn(confirmTag ?? '')}
              </Button>
            </div>
          </div>
        </Modal>
      )}
    </section>
  )
}
