import { useState, type KeyboardEvent, type MouseEvent } from 'react'
import { ContextMenu, type MenuItem } from '../../components/ContextMenu'
import { isNumericType, shortTypeName } from '../../components/data-grid/columns'
import { Icon } from '../../components/Icon'
import type { Column } from './api'
import { keyOf, rowKey, type Edits } from './edits'
import styles from './EditableGrid.module.css'

type Value = string | null

/** A cell being typed into: of a loaded row (by index) or of a new row (by id). */
type Editing = { row: number } | { newRow: number }

interface EditableGridProps {
  columns: Column[]
  rows: Value[][]
  firstRowNumber: number
  edits: Edits
  /** False for tables without a primary key: rows can't be identified to change them. */
  editable: boolean
  selected: Set<string>
  /** Click selects a row; ⌘/Ctrl-click adds or removes it. Keys are `rowKey()`s, or `new:<id>`. */
  onSelect: (id: string, additive: boolean) => void
  onEditCell: (row: number, column: number, value: Value) => void
  onEditNewCell: (id: number, column: number, value: Value) => void
  /** Puts a column of a new row back to its DEFAULT. */
  onUnsetNewCell: (id: number, column: number) => void
  /** Which way a column is sorted, for its header's arrow. */
  sortOf?: (column: string) => 'asc' | 'desc' | null
  /** A header click: sorts by that column, then the other way, then not at all. */
  onSort?: (column: string) => void
}

type CellState = { edited?: boolean; unset?: boolean; locked?: boolean }

const COMPUTED = 'Computed by the database, so it cannot be edited'

/** The table's rows with pending edits shown in place: new rows on top, deleted rows struck out. */
export function EditableGrid(props: EditableGridProps) {
  const { columns, rows, firstRowNumber, edits, editable, selected, onSelect } = props
  const [editing, setEditing] = useState<{ at: Editing; column: number } | null>(null)
  const [menu, setMenu] = useState<{
    x: number
    y: number
    at: Editing
    column: number
    state: CellState
  } | null>(null)
  const numeric = columns.map((c) => isNumericType(c.dataType))

  const setValue = (at: Editing, column: number, value: Value) => {
    if ('row' in at) props.onEditCell(at.row, column, value)
    else props.onEditNewCell(at.newRow, column, value)
  }

  const commit = (value: Value) => {
    if (!editing) return
    setValue(editing.at, editing.column, value)
    setEditing(null)
  }

  const menuItems = (at: Editing, column: number, state: CellState): MenuItem[] => [
    { label: 'Edit', onSelect: () => setEditing({ at, column }) },
    { label: 'Set NULL', onSelect: () => setValue(at, column, null) },
    ...('newRow' in at
      ? [
          {
            label: 'Use DEFAULT',
            disabled: state.unset,
            onSelect: () => props.onUnsetNewCell(at.newRow, column),
          },
        ]
      : [
          {
            label: 'Revert',
            disabled: !state.edited,
            // Setting the loaded value back drops the change.
            onSelect: () => props.onEditCell(at.row, column, rows[at.row][column]),
          },
        ]),
  ]

  const cell = (at: Editing, column: number, value: Value, state: CellState) => {
    const isEditing =
      editing?.column === column &&
      ('row' in at
        ? 'row' in editing.at && editing.at.row === at.row
        : 'newRow' in editing.at && editing.at.newRow === at.newRow)
    if (isEditing) {
      return (
        <td key={column} className={styles.editingCell}>
          <CellEditor initial={value} onCommit={commit} onCancel={() => setEditing(null)} />
        </td>
      )
    }
    const classes = [
      numeric[column] && styles.numeric,
      value === null && !state.unset && styles.null,
      state.unset && styles.unset,
      state.edited && styles.edited,
    ]
    return (
      <td
        key={column}
        className={classes.filter(Boolean).join(' ') || undefined}
        title={value ?? undefined}
        onDoubleClick={editable && !state.locked ? () => setEditing({ at, column }) : undefined}
        onContextMenu={
          editable && !state.locked
            ? (event: MouseEvent) => {
                event.preventDefault()
                setMenu({ x: event.clientX, y: event.clientY, at, column, state })
              }
            : undefined
        }
        data-menu-open={(menu && sameCell(menu, at, column)) || undefined}
      >
        {state.unset ? 'DEFAULT' : (value ?? 'NULL')}
      </td>
    )
  }

  return (
    <>
      <table className={styles.grid}>
        <thead>
          <tr>
            <th className={styles.rowNumber}>#</th>
            {columns.map((column) => {
              const sorted = props.sortOf?.(column.name) ?? null
              return (
                <th
                  key={column.name}
                  title={column.generated ? `${column.dataType} · ${COMPUTED}` : column.dataType}
                  aria-sort={sorted === 'asc' ? 'ascending' : sorted === 'desc' ? 'descending' : undefined}
                >
                  <button
                    type="button"
                    className={styles.header}
                    disabled={!props.onSort}
                    onClick={() => props.onSort?.(column.name)}
                  >
                    {column.primaryKey && <Icon name="key" size={13} className={styles.key} />}
                    {column.name}
                    {column.generated && <span className={styles.computed}>auto</span>}
                    <span className={styles.type}>{shortTypeName(column.dataType)}</span>
                    {props.onSort && (
                      // A click sorts ascending, then descending, then not at all.
                      <Icon
                        name={sorted === 'asc' ? 'chevronUp' : sorted === 'desc' ? 'chevronDown' : 'sortable'}
                        size={12}
                        className={sorted ? styles.sorted : styles.sortable}
                        aria-hidden="true"
                      />
                    )}
                  </button>
                </th>
              )
            })}
          </tr>
        </thead>
        <tbody>
          {edits.inserts.map((row) => {
            const id = `new:${row.id}`
            return (
              <tr key={id} className={styles.inserted} aria-selected={selected.has(id)}>
                <td
                  className={styles.rowNumber}
                  onClick={(event) => onSelect(id, event.metaKey || event.ctrlKey)}
                >
                  +
                </td>
                {columns.map((column, c) =>
                  cell({ newRow: row.id }, c, row.values[column.name] ?? null, {
                    edited: column.name in row.values,
                    unset: !(column.name in row.values),
                    locked: column.generated,
                  }),
                )}
              </tr>
            )
          })}
          {rows.map((row, r) => {
            const id = rowKey(keyOf(row, columns))
            const deleted = edits.deletes.has(id)
            const changed = edits.updates.get(id)?.values ?? {}
            return (
              <tr
                key={id + r}
                className={deleted ? styles.deleted : undefined}
                aria-selected={selected.has(id)}
              >
                <td
                  className={styles.rowNumber}
                  onClick={editable ? (event) => onSelect(id, event.metaKey || event.ctrlKey) : undefined}
                >
                  {deleted ? '−' : firstRowNumber + r}
                </td>
                {columns.map((column, c) =>
                  deleted
                    ? cell({ row: r }, c, row[c], { locked: true })
                    : cell({ row: r }, c, column.name in changed ? changed[column.name] : row[c], {
                        edited: column.name in changed,
                        locked: column.generated,
                      }),
                )}
              </tr>
            )
          })}
        </tbody>
      </table>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems(menu.at, menu.column, menu.state)}
          onClose={() => setMenu(null)}
        />
      )}
    </>
  )
}

function sameCell(menu: { at: Editing; column: number }, at: Editing, column: number): boolean {
  if (menu.column !== column) return false
  return 'row' in at
    ? 'row' in menu.at && menu.at.row === at.row
    : 'newRow' in menu.at && menu.at.newRow === at.newRow
}

/** Enter or leaving the field keeps the text; Escape cancels. NULL is set from the cell's menu. */
function CellEditor({
  initial,
  onCommit,
  onCancel,
}: {
  initial: Value
  onCommit: (value: Value) => void
  onCancel: () => void
}) {
  const [text, setText] = useState(initial ?? '')
  // Untouched text is no change: just opening a NULL or DEFAULT cell must not store ''.
  const finish = () => (text === (initial ?? '') ? onCancel() : onCommit(text))
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Enter') finish()
    else if (event.key === 'Escape') onCancel()
    else if ((event.metaKey || event.ctrlKey) && event.key === 's') {
      // Save this cell first; applying now would leave the text being typed out.
      // A second ⌘S applies everything.
      event.preventDefault()
      event.stopPropagation()
      finish()
    }
  }
  return (
    <span className={styles.editor}>
      <input
        autoFocus
        // Selected, so typing replaces the value and arrow keys still allow small fixes.
        onFocus={(event) => event.target.select()}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        onBlur={finish}
      />
    </span>
  )
}
