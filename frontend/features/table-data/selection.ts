/**
 * How a click picks cells: alone, added or removed (⌘/Ctrl), or a block from
 * the last one (Shift). `start` begins a drag: that cell alone, as the anchor.
 */
export type SelectMode = 'single' | 'toggle' | 'range' | 'start'

/** A cell by its row's selection key (a `rowKey()`, or `new:<id>`) and its column. */
export interface CellRef {
  row: string
  column: number
}

export interface Selection {
  /** `cellKey`s of the selected cells. */
  cells: Set<string>
  /** Where a Shift-click or a drag starts its block. */
  anchor: CellRef | null
}

export const NO_SELECTION: Selection = { cells: new Set(), anchor: null }

// Joins a row key and a column; row keys never contain this control character.
const SEP = '\u001f'

export function cellKey(cell: CellRef): string {
  return `${cell.row}${SEP}${cell.column}`
}

function cellOf(key: string): CellRef {
  const cut = key.lastIndexOf(SEP)
  return { row: key.slice(0, cut), column: Number(key.slice(cut + 1)) }
}

/**
 * The selection after a click on `target`. `order` is the rows as the grid
 * shows them; `wholeRow` picks every one of `columns` columns (a click on a
 * row's number).
 */
export function select(
  current: Selection,
  target: CellRef,
  mode: SelectMode,
  order: string[],
  columns: number,
  wholeRow = false,
): Selection {
  const span = (from: number, to: number) => {
    const [start, end] = from <= to ? [from, to] : [to, from]
    return Array.from({ length: end - start + 1 }, (_, i) => start + i)
  }
  const allColumns = span(0, columns - 1)
  const cellsOf = (row: string) =>
    (wholeRow ? allColumns : [target.column]).map((column) => cellKey({ row, column }))

  switch (mode) {
    case 'start':
    case 'single':
      return { cells: new Set(cellsOf(target.row)), anchor: target }
    case 'toggle': {
      const cells = new Set(current.cells)
      const keys = cellsOf(target.row)
      const all = keys.every((key) => cells.has(key))
      for (const key of keys) {
        if (all) cells.delete(key)
        else cells.add(key)
      }
      return { cells, anchor: target }
    }
    case 'range': {
      const anchor = current.anchor ?? target
      const from = order.indexOf(anchor.row)
      const to = order.indexOf(target.row)
      if (from < 0 || to < 0) return { cells: new Set(cellsOf(target.row)), anchor: target }
      const rows = span(from, to).map((index) => order[index])
      const picked = wholeRow ? allColumns : span(anchor.column, target.column)
      return {
        cells: new Set(rows.flatMap((row) => picked.map((column) => cellKey({ row, column })))),
        anchor,
      }
    }
  }
}

/** The selected cells, in the grid's order: by row, then by column. */
export function selectedCells(selection: Selection, order: string[]): CellRef[] {
  const position = new Map(order.map((row, index) => [row, index]))
  return [...selection.cells]
    .map(cellOf)
    .filter((cell) => position.has(cell.row))
    .sort((a, b) => position.get(a.row)! - position.get(b.row)! || a.column - b.column)
}

/** The rows with a selected cell, in the grid's order. */
export function selectedRows(selection: Selection, order: string[]): string[] {
  const rows = new Set(selectedCells(selection, order).map((cell) => cell.row))
  return order.filter((row) => rows.has(row))
}

/** The mode a click's modifier keys ask for; ⌘ on macOS, Ctrl elsewhere. */
export function modeOf(event: { shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }): SelectMode {
  if (event.shiftKey) return 'range'
  return event.metaKey || event.ctrlKey ? 'toggle' : 'single'
}

/**
 * The selected cells as tab-separated text for a spreadsheet: one line per
 * row with a selected cell, one field per column with one, empty where a
 * cell of that block isn't selected. A value with a tab, a line break or a
 * quote is quoted.
 */
export function cellsAsTsv(
  selection: Selection,
  order: string[],
  valueOf: (cell: CellRef) => string | null,
): string {
  const cells = selectedCells(selection, order)
  const columns = [...new Set(cells.map((cell) => cell.column))].sort((a, b) => a - b)
  const field = (value: string | null) => {
    const text = value ?? 'NULL'
    return /[\t\n\r"]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text
  }
  return selectedRows(selection, order)
    .map((row) =>
      columns
        .map((column) =>
          selection.cells.has(cellKey({ row, column })) ? field(valueOf({ row, column })) : '',
        )
        .join('\t'),
    )
    .join('\n')
}
