import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent,
  type PointerEvent,
} from 'react'
import { ContextMenu, type MenuItem } from '../../components/ContextMenu'
import { isNumericType, isUuidType, shortTypeName } from '../../components/data-grid/columns'
import { GutterGap, RowSheet } from '../../components/data-grid/RowGutter'
import { Icon } from '../../components/Icon'
import { drawnRows, useWindowedRows } from '../../components/useWindowedRows'
import { keepCursor } from '../../lib/cursor'
import { useStableHandlers } from '../../lib/useStableHandlers'
import { useMessages } from '../../lib/i18n/useLanguage'
import { isRelationalDialect, type SqlDialect } from '../../domain/connection'
import type { TableRef } from '../../domain/table'
import type { Column } from './api'
import { cellAt, DEFAULT, isDefault, rowKeys, type Cell, type CellValue, type Edits } from './edits'
import { messages } from './messages'
import { filtersByValue, rowAsInsert, rowAsJson, valueCondition } from './cellActions'
import {
  cellKey,
  cellsAsTsv,
  modeOf,
  selectedRows as rowsOfSelection,
  type CellRef,
  type Selection,
  type SelectMode,
} from './selection'
import { choicesOf } from './choices'
import { formatTemporal, temporalKindOf, temporalOf } from './temporal'
import {
  GridRow,
  RowNumber,
  type CellState,
  type Editing,
  type GridRowActions,
  type GridShape,
  type Value,
} from './GridRow'
import styles from './EditableGrid.module.css'

interface EditableGridProps {
  columns: Column[]
  rows: Value[][]
  firstRowNumber: number
  edits: Edits
  /** False for tables without a primary key: rows can't be identified to change them. */
  editable: boolean
  /** The selected cells; the rows they are in count as selected for row actions. */
  selection: Selection
  /**
   * Picks cells: a click alone, ⌘/Ctrl-click adds or removes, Shift-click or a
   * drag selects a block. On a row's number (`wholeRow`) it picks the whole row.
   */
  onSelect: (cell: CellRef, mode: SelectMode, wholeRow?: boolean) => void
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

const MIN_WIDTH = 48
const NO_WIDTHS: Record<string, number> = {}
/** A row's height in the style sheet: a very large page draws only the rows in sight. */
const ROW_HEIGHT = 26
const MOVES: Record<string, [number, number]> = {
  ArrowUp: [-1, 0],
  ArrowDown: [1, 0],
  ArrowLeft: [0, -1],
  ArrowRight: [0, 1],
}

/** The table's rows with pending edits shown in place: new rows on top, deleted rows struck out. */
export function EditableGrid(props: EditableGridProps) {
  const { columns, rows, firstRowNumber, edits, editable, selection, onSelect, active, onActivate } = props
  const m = useMessages(messages)
  const [editing, setEditing] = useState<{ at: Editing; column: number } | null>(null)
  const table = useRef<HTMLTableElement>(null)
  // New rows first, then the loaded ones: the rows by position, of which those in sight are drawn.
  const rowCount = edits.inserts.length + rows.length
  const {
    ref: scroller,
    onScroll,
    viewHeight,
    start,
    end,
  } = useWindowedRows(rowCount, ROW_HEIGHT, columns.length + 1)
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

  /**
   * Picks a cell and selects it, so row actions reach the cell the grid shows
   * as picked. `mode` says how the selection takes it; without one it stays as
   * it is when the cell is already part of it (a menu opened on a selected block).
   */
  const pick = (cell: Cell, mode?: SelectMode) => {
    onActivate(cell)
    // Cells of tables without a key can't be told apart once edited, so they aren't selected.
    if (!editable && !cell.row.startsWith('new:')) return
    const ref = { row: selectionRow(cell.row), column: cell.column }
    if (mode) onSelect(ref, mode)
    else if (!selection.cells.has(cellKey(ref))) onSelect(ref, 'single')
  }

  /** Arrow keys move the picked cell, new rows first as they are shown; with Shift they grow the selection. */
  const move = ([down, right]: [number, number], extend: boolean) => {
    if (!active) return
    const order = [...edits.inserts.map((r) => `new:${r.id}`), ...rows.map((_, r) => `row:${r}`)]
    const at = order.indexOf(active.row)
    const row = order[Math.min(order.length - 1, Math.max(0, at + down))]
    const column = Math.min(columns.length - 1, Math.max(0, active.column + right))
    pick({ row, column }, extend ? 'range' : 'single')
    // Once drawn, bring it into view past the sticky header and row numbers.
    requestAnimationFrame(() => {
      const cell = table.current?.querySelector('td[data-active]')
      if (cell) cell.scrollIntoView({ block: 'nearest', inline: 'nearest' })
      // Its row isn't drawn (the grid was scrolled away from it): go to where it is.
      else table.current?.parentElement?.scrollTo({ top: order.indexOf(row) * ROW_HEIGHT })
    })
  }

  // Widths the user dragged a column to; the others keep the one that fit the
  // rows drawn when the page loaded. A very large page draws only the rows in
  // sight, so a column fitted to them as they come and go would change width
  // while scrolling.
  const [widths, setWidths] = useState<Record<string, number>>({})
  const [fit, setFit] = useState<{ rows: Value[][]; widths: Record<string, number> }>({ rows, widths: {} })
  const fitted = fit.rows === rows ? fit.widths : NO_WIDTHS
  useLayoutEffect(() => {
    // Not while the tab is hidden: nothing has a size to measure.
    if (viewHeight === 0) return
    const measured: Record<string, number> = {}
    for (const header of table.current?.querySelectorAll<HTMLElement>('th[data-column]') ?? []) {
      const name = header.dataset.column!
      if (fitted[name] === undefined && widths[name] === undefined) {
        measured[name] = Math.ceil(header.getBoundingClientRect().width)
      }
    }
    // What the browser laid out is only known here, before it is painted.
    // eslint-disable-next-line react/set-state-in-effect
    if (Object.keys(measured).length > 0) setFit({ rows, widths: { ...fitted, ...measured } })
  }, [viewHeight, rows, columns, fitted, widths])
  const sized = useMemo(
    () =>
      columns.map((c): CSSProperties | undefined => {
        const width = widths[c.name] ?? fitted[c.name]
        return width === undefined
          ? undefined
          : { width, minWidth: width, maxWidth: width, boxSizing: 'border-box' }
      }),
    [columns, widths, fitted],
  )

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

  /** A double click on the edge fits the column again, to the rows in sight now. */
  const resetWidth = (name: string) => {
    const without = (current: Record<string, number>) => {
      const next = { ...current }
      delete next[name]
      return next
    }
    setWidths(without)
    setFit((current) => ({ ...current, widths: without(current.widths) }))
  }

  const setValue = (at: Editing, column: number, value: CellValue) => {
    if ('row' in at) props.onEditCell(at.row, column, value)
    // A new row's column at its DEFAULT is one the INSERT leaves out.
    else if (isDefault(value)) props.onUnsetNewCell(at.newRow, column)
    else props.onEditNewCell(at.newRow, column, value)
  }

  // What is the same for every cell of a row or of a column is worked out once, not per cell.
  const rowIds = useMemo(() => rowKeys(rows, columns), [rows, columns])
  const columnInfo = props.columnInfo
  const editors = useMemo(
    () =>
      columns.map((column) => ({
        temporalKind: temporalKindOf(column.dataType),
        choices: choicesOf(column.dataType, columnInfo?.[column.name]?.options),
      })),
    [columns, columnInfo],
  )
  /** The selection's key of a cell's row: its primary key, or `new:<id>`. */
  const rowIdOf = (at: Editing) => ('row' in at ? rowIds[at.row] : `new:${at.newRow}`)
  /** The key a `Cell` names a row with: `row:<index>` or `new:<id>`. */
  const cellIdOf = (at: Editing) => ('row' in at ? `row:${at.row}` : `new:${at.newRow}`)
  /** The selection's key, from the row a `Cell` names. */
  const selectionRow = (row: string) => (row.startsWith('new:') ? row : rowIds[Number(row.slice(4))])

  // The rows as shown, by selection key, and the key cellAt reads each with.
  const { order, cellRow } = useMemo(() => {
    const shown = [
      ...edits.inserts.map((row) => ({ select: `new:${row.id}`, cell: `new:${row.id}` })),
      ...rowIds.map((id, r) => ({ select: id, cell: `row:${r}` })),
    ]
    return {
      order: shown.map((row) => row.select),
      cellRow: new Map(shown.map((row) => [row.select, row.cell])),
    }
  }, [edits.inserts, rowIds])
  const selectedRowSet = useMemo(() => new Set(rowsOfSelection(selection, order)), [selection, order])
  // The rows whose every cell is selected, as their number picks them: drawn selected as a whole.
  const wholeRows = useMemo(
    () =>
      new Set(
        selection.cells.size < columns.length
          ? []
          : order.filter((id) =>
              columns.every((_, column) => selection.cells.has(cellKey({ row: id, column }))),
            ),
      ),
    [selection, order, columns],
  )
  const valueAt = (cell: CellRef) =>
    cellAt(edits, rows, columns, { row: cellRow.get(cell.row) ?? '', column: cell.column })?.value ?? null

  /** The rows with a selected cell, whole and as they stand (edits included), in the grid's order. */
  const selectedRows = (): Value[][] =>
    rowsOfSelection(selection, order).map((row) => columns.map((_, column) => valueAt({ row, column })))

  // A drag over the row numbers selects whole rows; it ends wherever the button is let go.
  const dragging = useRef(false)
  // A drag that starts on a cell: that cell, until the pointer reaches another one.
  const cellDrag = useRef<{ start: CellRef; moved: boolean } | null>(null)
  useEffect(() => {
    const stop = () => {
      dragging.current = false
      cellDrag.current = null
    }
    window.addEventListener('pointerup', stop)
    return () => window.removeEventListener('pointerup', stop)
  }, [])

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

    // Several selected cells, this one among them: a change reaches them all.
    const cellCount = selection.cells.size
    const across = cellCount > 1 && selection.cells.has(cellKey({ row: rowIdOf(at), column })) ? cellCount : 0
    const rowCount = selectedRowSet.size
    const forAll = (label: string) => (across ? m.acrossCells(label, across) : label)
    const copySelection: MenuItem[] = across
      ? [
          {
            label: m.copySelectedCells(across),
            onSelect: () => write(cellsAsTsv(selection, order, valueAt)),
          },
          {
            label: m.copySelectedJson(rowCount),
            onSelect: () =>
              write(
                `[${selectedRows()
                  .map((values) => rowAsJson(names, values))
                  .join(',\n')}]`,
              ),
          },
          ...(dialect && table && isRelationalDialect(dialect)
            ? [
                {
                  label: m.copySelectedInsert(rowCount),
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
        ...(dialect && table && isRelationalDialect(dialect)
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

  const numeric = useMemo(() => columns.map((c) => isNumericType(c.dataType)), [columns])
  const linkOf = props.linkOf
  const shape = useMemo<GridShape>(
    () => ({ columns, sized, numeric, editors, columnInfo, editable, linkOf }),
    [columns, sized, numeric, editors, columnInfo, editable, linkOf],
  )
  const actions: GridRowActions = useStableHandlers({
    pressCell: (at: Editing, column: number, event: PointerEvent) => {
      // Cells of tables without a key can't be told apart once edited, so they aren't selected.
      if (event.button !== 0 || modeOf(event) !== 'single' || !(editable || 'newRow' in at)) return
      cellDrag.current = { start: { row: rowIdOf(at), column }, moved: false }
    },
    enterCell: (at: Editing, column: number) => {
      const drag = cellDrag.current
      if (!drag) return
      const here = { row: rowIdOf(at), column }
      if (!drag.moved && cellKey(here) === cellKey(drag.start)) return
      if (!drag.moved) {
        drag.moved = true
        onSelect(drag.start, 'start')
        // Text selection would otherwise follow the pointer across the cells.
        window.getSelection()?.removeAllRanges()
      }
      onSelect(here, 'range')
    },
    clickCell: (at: Editing, column: number, event: MouseEvent) =>
      pick({ row: cellIdOf(at), column }, modeOf(event)),
    editCell: (at: Editing, column: number) => setEditing({ at, column }),
    openMenu: (at: Editing, column: number, state: CellState, event: MouseEvent) => {
      event.preventDefault()
      pick({ row: cellIdOf(at), column })
      setMenu({ x: event.clientX, y: event.clientY, at, column, state })
    },
    pressRowNumber: (rowId: string, event: PointerEvent) => {
      if (event.button !== 0) return
      event.preventDefault()
      dragging.current = true
      onSelect({ row: rowId, column: 0 }, modeOf(event), true)
    },
    enterRowNumber: (rowId: string) => {
      if (dragging.current) onSelect({ row: rowId, column: 0 }, 'range', true)
    },
    commit,
    cancelEdit: stopEditing,
  })

  /** How the selection, the editor and the menu reach the row drawn at `at`. */
  const marksOf = (at: Editing) => {
    const rowId = rowIdOf(at)
    const whole = wholeRows.has(rowId)
    const cellId = cellIdOf(at)
    const here = (place: { at: Editing } | null) => place !== null && cellIdOf(place.at) === cellId
    return {
      whole,
      activeColumn: active?.row === cellId ? active.column : null,
      selectedColumns:
        selection.cells.size > 1 && !whole
          ? columns
              .flatMap((_, column) => (selection.cells.has(cellKey({ row: rowId, column })) ? [column] : []))
              .join(',')
          : '',
      editingColumn: editing && here(editing) ? editing.column : null,
      menuColumn: menu && here(menu) ? menu.column : null,
    }
  }

  /** The number beside the row drawn at `index`, in the gutter. */
  const drawNumber = (index: number) => {
    if (index < edits.inserts.length) {
      const rowId = `new:${edits.inserts[index].id}`
      return (
        <RowNumber
          key={rowId}
          rowId={rowId}
          label="+"
          state="inserted"
          whole={wholeRows.has(rowId)}
          picks
          actions={actions}
        />
      )
    }
    const r = index - edits.inserts.length
    const rowId = rowIds[r]
    const deleted = edits.deletes.has(rowId)
    return (
      <RowNumber
        key={rowId + r}
        rowId={rowId}
        label={deleted ? '−' : firstRowNumber + r}
        state={deleted ? 'deleted' : 'loaded'}
        whole={wholeRows.has(rowId)}
        picks={editable}
        actions={actions}
      />
    )
  }

  const drawRow = (index: number) => {
    if (index < edits.inserts.length) {
      const insert = edits.inserts[index]
      const at = { newRow: insert.id }
      return (
        <GridRow
          key={`new:${insert.id}`}
          kind="inserted"
          insert={insert}
          {...marksOf(at)}
          shape={shape}
          actions={actions}
        />
      )
    }
    const r = index - edits.inserts.length
    const id = rowIds[r]
    return (
      <GridRow
        key={id + r}
        kind="loaded"
        index={r}
        values={rows[r]}
        changed={edits.updates.get(id)?.values}
        deleted={edits.deletes.has(id)}
        {...marksOf({ row: r })}
        shape={shape}
        actions={actions}
      />
    )
  }

  // The row being edited stays drawn while out of sight, so what was typed in it isn't lost.
  const editingRow = !editing
    ? null
    : 'row' in editing.at
      ? edits.inserts.length + editing.at.row
      : edits.inserts.findIndex((row) => 'newRow' in editing.at && row.id === editing.at.newRow)

  const parts = drawnRows(start, end, editingRow, rowCount)

  return (
    <>
      <div ref={scroller} onScroll={onScroll} className={styles.scroller}>
        <RowSheet
          lastNumber={firstRowNumber + rows.length - 1}
          numbers={parts.map((part) =>
            'gap' in part ? (
              <GutterGap key={`gap:${part.from}`} rows={part.gap} rowHeight={ROW_HEIGHT} />
            ) : (
              drawNumber(part.index)
            ),
          )}
        >
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
              if (command && event.key === 'c' && selection.cells.size > 1) {
                // Several cells selected: they go to the clipboard as a spreadsheet's block.
                event.preventDefault()
                void navigator.clipboard.writeText(cellsAsTsv(selection, order, valueAt))
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
                move(MOVES[event.key], event.shiftKey)
              }
            }}
          >
            <thead>
              <tr>
                {columns.map((column, c) => {
                  const sorted = props.sortOf?.(column.name) ?? null
                  return (
                    <th
                      key={column.name}
                      data-column={column.name}
                      style={sized[c]}
                      title={column.generated ? `${column.dataType} · ${m.computed}` : column.dataType}
                      aria-sort={
                        sorted === 'asc' ? 'ascending' : sorted === 'desc' ? 'descending' : undefined
                      }
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
                          name={
                            sorted === 'asc' ? 'chevronUp' : sorted === 'desc' ? 'chevronDown' : 'sortable'
                          }
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
              {parts.map((part) =>
                'gap' in part ? (
                  // Stands in for the rows out of sight.
                  <tr key={`gap:${part.from}`} className={styles.gap} aria-hidden="true">
                    <td
                      colSpan={columns.length}
                      style={
                        { height: part.gap * ROW_HEIGHT, '--row-height': `${ROW_HEIGHT}px` } as CSSProperties
                      }
                    />
                  </tr>
                ) : (
                  drawRow(part.index)
                ),
              )}
            </tbody>
          </table>
        </RowSheet>
      </div>
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
