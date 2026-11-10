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
import { isNumericType, isUuidType, shortTypeName } from '../../components/data-grid/columns'
import { Icon } from '../../components/Icon'
import { keepCursor } from '../../lib/cursor'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { SqlDialect } from '../../domain/connection'
import type { TableRef } from '../../domain/table'
import type { Column } from './api'
import { cellAt, DEFAULT, isDefault, keyOf, rowKey, type Cell, type CellValue, type Edits } from './edits'
import { messages } from './messages'
import { filtersByValue, rowAsInsert, rowAsJson, valueCondition } from './cellActions'
import { modeOf, rowsAsTsv, type SelectMode } from './rowSelection'
import { ChoiceEditor } from './ChoiceEditor'
import { choicesOf } from './choices'
import { formatTemporal, temporalKindOf, temporalOf } from './temporal'
import { TemporalEditor } from './TemporalEditor'
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
  /**
   * Picks rows: a click alone, ⌘/Ctrl-click adds or removes, Shift-click or a
   * drag over the row numbers selects a block. Keys are `rowKey()`s, or `new:<id>`.
   */
  onSelect: (id: string, mode: SelectMode) => void
  onEditCell: (row: number, column: number, value: CellValue) => void
  onEditNewCell: (id: number, column: number, value: Value) => void
  /** Puts a column of a new row back to its DEFAULT. */
  onUnsetNewCell: (id: number, column: number) => void
  /** Which way a column is sorted, for its header's arrow. */
  sortOf?: (column: string) => 'asc' | 'desc' | null
  /** A header click: sorts by that column, then the other way, then not at all. */
  onSort?: (column: string) => void
  /** The picked cell; a click picks one, the arrow keys move it, Escape lets go. */
  active: Cell | null
  onActivate: (cell: Cell | null) => void
  /** Shift+Enter: shows the picked cell in the inspector. */
  onInspect?: () => void
  /** What the table's structure says of each column, by name, once it has loaded. */
  columnInfo?: Record<string, { nullable: boolean; default: string | null; options?: string[] }>
  /** Opens the row a foreign key column points to; `null` for other columns or NULL keys. */
  linkOf?: (row: number, column: number) => (() => void) | null
  /** The query language, for the conditions and statements the cell menu writes. */
  dialect?: SqlDialect
  /** The table, for the INSERT the cell menu copies. */
  table?: TableRef
  /** Narrows the rows by a condition on a cell's value; absent while changes are pending. */
  onNarrow?: (condition: string) => void
  /** Sorts by one column; absent while changes are pending. */
  onSortColumn?: (column: string, direction: 'asc' | 'desc') => void
  /** Adds a new row with a loaded row's values. */
  onDuplicateRow?: (row: number) => void
  /** Marks a loaded row for deletion, or keeps it again. */
  onDeleteRow?: (row: number) => void
}

const RELATIONAL_DIALECTS: SqlDialect[] = ['postgres', 'mysql', 'sqlite', 'sqlserver']

type CellState = { edited?: boolean; unset?: boolean; locked?: boolean }

const MIN_WIDTH = 48
const MOVES: Record<string, [number, number]> = {
  ArrowUp: [-1, 0],
  ArrowDown: [1, 0],
  ArrowLeft: [0, -1],
  ArrowRight: [0, 1],
}

/** The table's rows with pending edits shown in place: new rows on top, deleted rows struck out. */
export function EditableGrid(props: EditableGridProps) {
  const { columns, rows, firstRowNumber, edits, editable, selected, onSelect, active, onActivate } = props
  const m = useMessages(messages)
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

  /** Arrow keys move the picked cell, new rows first as they are shown. */
  const move = ([down, right]: [number, number]) => {
    if (!active) return
    const order = [...edits.inserts.map((r) => `new:${r.id}`), ...rows.map((_, r) => `row:${r}`)]
    const at = order.indexOf(active.row)
    const row = order[Math.min(order.length - 1, Math.max(0, at + down))]
    const column = Math.min(columns.length - 1, Math.max(0, active.column + right))
    onActivate({ row, column })
    // Once drawn, bring it into view past the sticky header and row numbers.
    requestAnimationFrame(() =>
      table.current
        ?.querySelector('td[data-active]')
        ?.scrollIntoView({ block: 'nearest', inline: 'nearest' }),
    )
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

  const setValue = (at: Editing, column: number, value: CellValue) => {
    if ('row' in at) props.onEditCell(at.row, column, value)
    // A new row's column at its DEFAULT is one the INSERT leaves out.
    else if (isDefault(value)) props.onUnsetNewCell(at.newRow, column)
    else props.onEditNewCell(at.newRow, column, value)
  }

  /** The selection's key of a cell's row: its primary key, or `new:<id>`. */
  const rowIdOf = (at: Editing) => ('row' in at ? rowKey(keyOf(rows[at.row], columns)) : `new:${at.newRow}`)

  /** The selected rows' values as they stand (edits included), in the grid's order. */
  const selectedRows = (): Value[][] => {
    const ids = [
      ...edits.inserts.map((row) => ({ select: `new:${row.id}`, cell: `new:${row.id}` })),
      ...rows.map((row, r) => ({ select: rowKey(keyOf(row, columns)), cell: `row:${r}` })),
    ].filter(({ select }) => selected.has(select))
    return ids.map(({ cell }) =>
      columns.map((_, c) => cellAt(edits, rows, columns, { row: cell, column: c })?.value ?? null),
    )
  }

  // A drag over the row numbers selects a block; it ends wherever the button is let go.
  const dragging = useRef(false)
  useEffect(() => {
    const stop = () => {
      dragging.current = false
    }
    window.addEventListener('pointerup', stop)
    return () => window.removeEventListener('pointerup', stop)
  }, [])
  const rowNumberProps = (id: string) => ({
    onPointerDown: (event: PointerEvent) => {
      if (event.button !== 0) return
      event.preventDefault()
      dragging.current = true
      onSelect(id, modeOf(event))
    },
    onPointerEnter: () => dragging.current && onSelect(id, 'range'),
  })

  /** A cell's text for the clipboard: its value, or NULL or DEFAULT spelled out. */
  const cellText = (cell: Cell) => {
    const value = cellAt(edits, rows, columns, cell)
    return value && (value.unset ? 'DEFAULT' : (value.value ?? 'NULL'))
  }

  /** Puts the clipboard's text in a cell; WebKit may ask first with its own Paste button. */
  const paste = (at: Editing, column: number) =>
    void navigator.clipboard
      .readText()
      .then((text) => setValue(at, column, text))
      .catch(() => {})

  const commit = (value: Value) => {
    if (!editing) return
    setValue(editing.at, editing.column, value)
    stopEditing()
  }

  /** Whether the cell takes a pasted value: a table that can be edited, and a column and row that can change. */
  const pasteable = (cell: Cell) =>
    editable && !columns[cell.column]?.generated && cellAt(edits, rows, columns, cell)?.deleted === false

  /** The cell's menu: copying, filtering and sorting always; the changes its column and row allow. */
  const menuItems = (at: Editing, column: number, state: CellState): MenuItem[] => {
    const id = 'row' in at ? `row:${at.row}` : `new:${at.newRow}`
    const { name, dataType } = columns[column]
    const names = columns.map((c) => c.name)
    // The row as it stands, edits included; DEFAULT reads as NULL.
    const current = () =>
      columns.map((_, c) => cellAt(edits, rows, columns, { row: id, column: c })?.value ?? null)
    const value = cellAt(edits, rows, columns, { row: id, column })?.value ?? null
    const write = (text: string) => void navigator.clipboard.writeText(text)
    const { dialect, table } = props

    // Several selected rows, this one among them: a change reaches them all.
    const across = selected.size > 1 && selected.has(rowIdOf(at)) ? selected.size : 0
    const forAll = (label: string) => (across ? m.acrossRows(label, across) : label)
    const copySelection: MenuItem[] = across
      ? [
          { label: m.copySelectedRows(across), onSelect: () => write(rowsAsTsv(selectedRows())) },
          {
            label: m.copySelectedJson(across),
            onSelect: () =>
              write(
                `[${selectedRows()
                  .map((values) => rowAsJson(names, values))
                  .join(',\n')}]`,
              ),
          },
          ...(dialect && table && RELATIONAL_DIALECTS.includes(dialect)
            ? [
                {
                  label: m.copySelectedInsert(across),
                  onSelect: () =>
                    write(
                      selectedRows()
                        .map((values) => rowAsInsert(dialect, table, names, values))
                        .join('\n'),
                    ),
                },
              ]
            : []),
        ]
      : []

    const copy: MenuItem = {
      label: m.copyMenu,
      icon: 'duplicate',
      items: [
        ...copySelection,
        {
          label: m.copyValue,
          onSelect: () => {
            const text = cellText({ row: id, column })
            if (text !== null) write(text)
          },
        },
        { label: m.copyColumnName, onSelect: () => write(name) },
        ...(dialect && filtersByValue(dialect)
          ? [{ label: m.copyCondition, onSelect: () => write(valueCondition(dialect, name, 'equal', value)) }]
          : []),
        { label: m.copyRowJson, onSelect: () => write(rowAsJson(names, current())) },
        ...(dialect && table && RELATIONAL_DIALECTS.includes(dialect)
          ? [{ label: m.copyRowInsert, onSelect: () => write(rowAsInsert(dialect, table, names, current())) }]
          : []),
      ],
    }

    const narrow = props.onNarrow
    const sort = props.onSortColumn
    const filter: MenuItem[] =
      narrow || sort
        ? [
            {
              label: m.filterMenu,
              icon: 'search',
              items: [
                ...(narrow && dialect && filtersByValue(dialect)
                  ? (
                      [
                        [m.filterEqual, 'equal'],
                        [m.filterNotEqual, 'notEqual'],
                        [m.filterIsNull, 'isNull'],
                        [m.filterIsNotNull, 'isNotNull'],
                      ] as const
                    ).map(([label, test]) => ({
                      label,
                      onSelect: () => narrow(valueCondition(dialect, name, test, value)),
                    }))
                  : []),
                ...(sort
                  ? [
                      { label: m.sortAscending, onSelect: () => sort(name, 'asc' as const) },
                      { label: m.sortDescending, onSelect: () => sort(name, 'desc' as const) },
                    ]
                  : []),
              ],
            },
          ]
        : []
    if (!editable || state.locked) return [copy, ...filter]

    // Until the structure loads, nothing is assumed about NULL; the server has the last word.
    const info = props.columnInfo?.[name]
    const temporal = temporalKindOf(dataType)
    const now = () =>
      setValue(
        at,
        column,
        formatTemporal(temporalOf(new Date()), temporal!, /datetimeoffset/i.test(dataType)),
      )
    const set: MenuItem = {
      label: forAll(m.setMenu),
      icon: 'edit',
      items: [
        ...((info?.nullable ?? true) ? [{ label: 'NULL', onSelect: () => setValue(at, column, null) }] : []),
        ...(info?.default
          ? [{ label: 'DEFAULT', disabled: state.unset, onSelect: () => setValue(at, column, DEFAULT) }]
          : []),
        ...(temporal ? [{ label: temporal === 'date' ? m.today : m.now, onSelect: now }] : []),
        ...(isUuidType(dataType)
          ? [{ label: m.generateUuid, onSelect: () => setValue(at, column, crypto.randomUUID()) }]
          : []),
      ],
    }
    const deleted = 'row' in at && cellAt(edits, rows, columns, { row: id, column })?.deleted
    const row: MenuItem[] =
      'row' in at && (props.onDuplicateRow || props.onDeleteRow)
        ? [
            {
              label: m.rowMenu,
              icon: 'table',
              items: [
                ...(props.onDuplicateRow
                  ? [{ label: m.duplicateRow, onSelect: () => props.onDuplicateRow!(at.row) }]
                  : []),
                ...(props.onDeleteRow
                  ? [
                      {
                        label: deleted ? m.restoreRow : m.deleteRow,
                        onSelect: () => props.onDeleteRow!(at.row),
                      },
                    ]
                  : []),
              ],
            },
          ]
        : []

    return [
      { label: forAll(m.edit), onSelect: () => setEditing({ at, column }) },
      copy,
      { label: forAll(m.paste), onSelect: () => paste(at, column) },
      ...(set.items!.length > 0 ? [set] : []),
      ...filter,
      ...row,
      ...('row' in at
        ? [
            {
              label: m.revert,
              disabled: !state.edited,
              // Setting the loaded value back drops the change.
              onSelect: () => props.onEditCell(at.row, column, rows[at.row][column]),
            },
          ]
        : []),
    ]
  }

  const cell = (at: Editing, column: number, value: Value, state: CellState) => {
    const isEditing =
      editing?.column === column &&
      ('row' in at
        ? 'row' in editing.at && editing.at.row === at.row
        : 'newRow' in editing.at && editing.at.newRow === at.newRow)
    const id = 'row' in at ? `row:${at.row}` : `new:${at.newRow}`
    const isActive = active?.row === id && active.column === column
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
          aria-label={m.openPointedRow(value)}
          title={m.openReferencedRow}
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
    const temporalKind = temporalKindOf(columns[column].dataType)
    const choices = choicesOf(columns[column].dataType, props.columnInfo?.[columns[column].name]?.options)
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
          {choices ? (
            <ChoiceEditor
              initial={value}
              choices={choices}
              nullable={props.columnInfo?.[columns[column].name]?.nullable ?? true}
              onCommit={commit}
              onCancel={stopEditing}
            />
          ) : temporalKind ? (
            <TemporalEditor
              initial={value}
              kind={temporalKind}
              offsetSpace={/datetimeoffset/i.test(columns[column].dataType)}
              nullable={props.columnInfo?.[columns[column].name]?.nullable ?? true}
              onCommit={commit}
              onCancel={stopEditing}
            />
          ) : (
            <CellEditor initial={value} onCommit={commit} onCancel={stopEditing} />
          )}
        </td>
      )
    }
    return (
      <td
        key={column}
        className={classes.filter(Boolean).join(' ') || undefined}
        style={sized[column]}
        title={value ?? undefined}
        onClick={(event) => {
          onActivate({ row: id, column })
          // Shift or ⌘/Ctrl on a cell picks rows too, where rows can be told apart.
          const mode = modeOf(event)
          if (mode !== 'single' && (editable || !('row' in at))) onSelect(rowIdOf(at), mode)
        }}
        onDoubleClick={editable && !state.locked ? () => setEditing({ at, column }) : undefined}
        onContextMenu={(event: MouseEvent) => {
          event.preventDefault()
          onActivate({ row: id, column })
          setMenu({ x: event.clientX, y: event.clientY, at, column, state })
        }}
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
          const text = active && cellText(active)
          const command = event.metaKey || event.ctrlKey
          if (command && event.key === 'c' && selected.size > 1) {
            // Several rows selected: they go to the clipboard as a spreadsheet's rows.
            event.preventDefault()
            void navigator.clipboard.writeText(rowsAsTsv(selectedRows()))
          } else if (command && event.key === 'c' && text !== null) {
            event.preventDefault()
            void navigator.clipboard.writeText(text)
          } else if (command && event.key === 'v' && active && pasteable(active)) {
            event.preventDefault()
            const [kind, index] = active.row.split(':')
            paste(kind === 'new' ? { newRow: Number(index) } : { row: Number(index) }, active.column)
          } else if (event.key === 'Enter' && event.shiftKey && props.onInspect) {
            event.preventDefault()
            props.onInspect()
          } else if (event.key === 'Escape') onActivate(null)
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
                  title={column.generated ? `${column.dataType} · ${m.computed}` : column.dataType}
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
                    {column.generated && <span className={styles.computed}>{m.computedBadge}</span>}
                    <span className={styles.type}>{shortTypeName(column.dataType)}</span>
                    {/* A click sorts ascending, then descending, then not at all. The mark stays
                        while sorting is off, so the column keeps its width. */}
                    <Icon
                      name={sorted === 'asc' ? 'chevronUp' : sorted === 'desc' ? 'chevronDown' : 'sortable'}
                      size={12}
                      className={sorted ? styles.sorted : styles.sortable}
                      aria-hidden="true"
                    />
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
                <td className={styles.rowNumber} {...rowNumberProps(id)}>
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
                <td className={styles.rowNumber} {...(editable ? rowNumberProps(id) : {})}>
                  {deleted ? '−' : firstRowNumber + r}
                </td>
                {columns.map((column, c) =>
                  deleted
                    ? cell({ row: r }, c, row[c], { locked: true })
                    : cell({ row: r }, c, column.name in changed ? textOf(changed[column.name]) : row[c], {
                        edited: column.name in changed,
                        unset: column.name in changed && isDefault(changed[column.name]),
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

/** A DEFAULT shows as an empty cell marked DEFAULT; other values as they are. */
function textOf(value: CellValue): Value {
  return isDefault(value) ? null : value
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
