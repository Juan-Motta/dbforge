import { useState, type KeyboardEvent } from 'react'
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
}

/** The table's rows with pending edits shown in place: new rows on top, deleted rows struck out. */
export function EditableGrid(props: EditableGridProps) {
  const { columns, rows, firstRowNumber, edits, editable, selected, onSelect } = props
  const [editing, setEditing] = useState<{ at: Editing; column: number } | null>(null)
  const numeric = columns.map((c) => isNumericType(c.dataType))

  const commit = (value: Value) => {
    if (!editing) return
    if ('row' in editing.at) props.onEditCell(editing.at.row, editing.column, value)
    else props.onEditNewCell(editing.at.newRow, editing.column, value)
    setEditing(null)
  }

  const cell = (
    at: Editing,
    column: number,
    value: Value,
    state: { edited?: boolean; unset?: boolean; locked?: boolean },
  ) => {
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
      >
        {state.unset ? 'DEFAULT' : (value ?? 'NULL')}
      </td>
    )
  }

  return (
    <table className={styles.grid}>
      <thead>
        <tr>
          <th className={styles.rowNumber}>#</th>
          {columns.map((column) => (
            <th key={column.name} title={column.dataType}>
              <span className={styles.header}>
                {column.primaryKey && <Icon name="key" size={13} className={styles.key} />}
                {column.name}
                <span className={styles.type}>{shortTypeName(column.dataType)}</span>
              </span>
            </th>
          ))}
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
                    }),
              )}
            </tr>
          )
        })}
      </tbody>
    </table>
  )
}

/** Enter or leaving the field keeps the text; Escape cancels; the NULL button stores NULL. */
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
      <button
        type="button"
        title="Set to NULL"
        // Before the input's blur, which would otherwise store the text first.
        onMouseDown={(event) => {
          event.preventDefault()
          onCommit(null)
        }}
      >
        NULL
      </button>
    </span>
  )
}
