import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
} from 'react'
import { ContextMenu, type MenuItem } from '../../components/ContextMenu'
import { isNumericType, shortTypeName } from '../../components/data-grid/columns'
import { Icon } from '../../components/Icon'
import { keepCursor } from '../../lib/cursor'
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
  /** Opens the row a foreign key column points to; `null` for other columns or NULL keys. */
  linkOf?: (row: number, column: number) => (() => void) | null
}

type CellState = { edited?: boolean; unset?: boolean; locked?: boolean }

const COMPUTED = 'Computed by the database, so it cannot be edited'
const MIN_WIDTH = 48
const MOVES: Record<string, [number, number]> = {
  ArrowUp: [-1, 0],
  ArrowDown: [1, 0],
  ArrowLeft: [0, -1],
  ArrowRight: [0, 1],
}

/** The table's rows with pending edits shown in place: new rows on top, deleted rows struck out. */
export function EditableGrid(props: EditableGridProps) {
  const { columns, rows, firstRowNumber, edits, editable, selected, onSelect } = props
  const [editing, setEditing] = useState<{ at: Editing; column: number } | null>(null)
  const table = useRef<HTMLTableElement>(null)
  // A grid closed while it had focus gives the pointer back to the usual rules.
  useEffect(() => () => keepCursor(false), [])
  /** Closes the editor and gives the keys back to the grid, so ⌘C still copies. */
  const stopEditing = () => {
    setEditing(null)
    table.current?.focus()
  }
  const [menu, setMenu] = useState<{
    x: number
    y: number
    at: Editing
    column: number
    state: CellState
  } | null>(null)
  // The cell a click picked out, as `row:<index>` or `new:<id>` and a column.
  const [active, setActive] = useState<{ id: string; column: number } | null>(null)
  // Another page or table: the picked cell is gone.
  const [shown, setShown] = useState(rows)
  if (shown !== rows) {
    setShown(rows)
    setActive(null)
  }

  /** Arrow keys move the picked cell, new rows first as they are shown. */
  const move = ([down, right]: [number, number]) => {
    if (!active) return
    const order = [...edits.inserts.map((r) => `new:${r.id}`), ...rows.map((_, r) => `row:${r}`)]
    const at = order.indexOf(active.id)
    const id = order[Math.min(order.length - 1, Math.max(0, at + down))]
    const column = Math.min(columns.length - 1, Math.max(0, active.column + right))
    setActive({ id, column })
    // Once drawn, bring it into view past the sticky header and row numbers.
    requestAnimationFrame(() =>
      table.current
        ?.querySelector('td[data-active]')
        ?.scrollIntoView({ block: 'nearest', inline: 'nearest' }),
    )
  }

  /** The picked cell's value as shown, edits included; `undefined` when none is picked. */
  const activeValue = (): Value | undefined => {
    if (!active) return undefined
    const name = columns[active.column]?.name
    if (name === undefined) return undefined
    if (active.id.startsWith('new:')) {
      const row = edits.inserts.find((r) => `new:${r.id}` === active.id)
      return row ? (row.values[name] ?? null) : undefined
    }
    const row = rows[Number(active.id.slice(4))]
    if (!row) return undefined
    const changed = edits.updates.get(rowKey(keyOf(row, columns)))?.values ?? {}
    return name in changed ? changed[name] : row[active.column]
  }
  const numeric = columns.map((c) => isNumericType(c.dataType))
  // Widths the user dragged a column to; the others fit their content.
  const [widths, setWidths] = useState<Record<string, number>>({})
  const sized = columns.map((c): CSSProperties | undefined => {
    const width = widths[c.name]
    return width === undefined
      ? undefined
      : { width, minWidth: width, maxWidth: width, boxSizing: 'border-box' }
  })

  /** Dragging a header's right edge sets that column's width. */
  const startResize = (name: string) => (event: PointerEvent<HTMLElement>) => {
    event.preventDefault()
    const handle = event.currentTarget
    const start = event.clientX
    const initial = handle.parentElement!.getBoundingClientRect().width
    handle.setPointerCapture(event.pointerId)
    const move = (e: globalThis.PointerEvent) =>
      setWidths((current) => ({
        ...current,
        [name]: Math.max(MIN_WIDTH, Math.round(initial + e.clientX - start)),
      }))
    const stop = () => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', stop)
      handle.removeEventListener('pointercancel', stop)
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', stop)
    handle.addEventListener('pointercancel', stop)
  }

  /** A double click on the edge fits the column to its content again. */
  const resetWidth = (name: string) =>
    setWidths((current) => {
      const next = { ...current }
      delete next[name]
      return next
    })

  const setValue = (at: Editing, column: number, value: Value) => {
    if ('row' in at) props.onEditCell(at.row, column, value)
    else props.onEditNewCell(at.newRow, column, value)
  }

  const commit = (value: Value) => {
    if (!editing) return
    setValue(editing.at, editing.column, value)
    stopEditing()
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
    const id = 'row' in at ? `row:${at.row}` : `new:${at.newRow}`
    const isActive = active?.id === id && active.column === column
    // Edited or new values may not exist yet in the referenced table: only loaded ones link.
    const link = 'row' in at && !state.edited ? (props.linkOf?.(at.row, column) ?? null) : null
    const classes = [
      numeric[column] && styles.numeric,
      value === null && !state.unset && styles.null,
      state.unset && styles.unset,
      state.edited && styles.edited,
    ]
    const content = link ? (
      <span className={styles.linked}>
        <span className={styles.linkedText}>{value}</span>
        <button
          type="button"
          className={styles.link}
          aria-label={`Open the row ${value} points to`}
          title="Open the referenced row"
          onClick={(event) => {
            event.stopPropagation()
            link()
          }}
          onDoubleClick={(event) => event.stopPropagation()}
        >
          <Icon name="external" size={12} />
        </button>
      </span>
    ) : state.unset ? (
      'DEFAULT'
    ) : (
      (value ?? 'NULL')
    )
    if (isEditing) {
      // The editor lies over the cell, which keeps its content (unseen) and so its size.
      return (
        <td
          key={column}
          className={[...classes, styles.editingCell].filter(Boolean).join(' ')}
          style={sized[column]}
        >
          <span className={styles.hidden} aria-hidden="true">
            {content}
          </span>
          <CellEditor initial={value} onCommit={commit} onCancel={stopEditing} />
        </td>
      )
    }
    return (
      <td
        key={column}
        className={classes.filter(Boolean).join(' ') || undefined}
        style={sized[column]}
        title={value ?? undefined}
        onClick={() => setActive({ id, column })}
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
        data-active={isActive || undefined}
      >
        {content}
      </td>
    )
  }

  return (
    <>
      <table
        ref={table}
        className={styles.grid}
        // Focusable, so ⌘C copies the picked cell.
        tabIndex={-1}
        // While the grid has the keys, moving with the arrows keeps the pointer in sight.
        onFocus={(event) => event.target === event.currentTarget && keepCursor(true)}
        onBlur={(event) => event.target === event.currentTarget && keepCursor(false)}
        onKeyDown={(event) => {
          // Keys typed into the cell editor are its own.
          if (event.target instanceof HTMLInputElement) return
          const value = activeValue()
          if ((event.metaKey || event.ctrlKey) && event.key === 'c' && value !== undefined) {
            event.preventDefault()
            void navigator.clipboard.writeText(value ?? 'NULL')
          } else if (event.key === 'Escape') setActive(null)
          else if (active && event.key in MOVES) {
            event.preventDefault()
            move(MOVES[event.key])
          }
        }}
      >
        <thead>
          <tr>
            <th className={styles.rowNumber}>#</th>
            {columns.map((column, c) => {
              const sorted = props.sortOf?.(column.name) ?? null
              return (
                <th
                  key={column.name}
                  style={sized[c]}
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
                    <span className={styles.name}>{column.name}</span>
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
                  <span
                    className={styles.resize}
                    aria-hidden="true"
                    onPointerDown={startResize(column.name)}
                    onDoubleClick={() => resetWidth(column.name)}
                  />
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
