/** How a click picks rows: alone, added or removed (⌘/Ctrl), or a block from the last one (Shift). */
export type SelectMode = 'single' | 'toggle' | 'range'

export interface RowSelection {
  selected: Set<string>
  /** Where a Shift-click or a drag starts its block: the last row picked alone or toggled. */
  anchor: string | null
}

export const NO_SELECTION: RowSelection = { selected: new Set(), anchor: null }

/** The selection after a click on row `id`; `order` is the rows as the grid shows them. */
export function selectRows(
  current: RowSelection,
  id: string,
  mode: SelectMode,
  order: string[],
): RowSelection {
  switch (mode) {
    case 'single': {
      // Clicking the only selected row again clears the selection.
      const alone = current.selected.size === 1 && current.selected.has(id)
      return { selected: alone ? new Set() : new Set([id]), anchor: alone ? null : id }
    }
    case 'toggle': {
      const selected = new Set(current.selected)
      if (selected.has(id)) selected.delete(id)
      else selected.add(id)
      return { selected, anchor: id }
    }
    case 'range': {
      const from = order.indexOf(current.anchor ?? id)
      const to = order.indexOf(id)
      if (from < 0 || to < 0) return { selected: new Set([id]), anchor: id }
      const [start, end] = from <= to ? [from, to] : [to, from]
      return { selected: new Set(order.slice(start, end + 1)), anchor: current.anchor ?? id }
    }
  }
}

/** The mode a click's modifier keys ask for; ⌘ on macOS, Ctrl elsewhere. */
export function modeOf(event: { shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }): SelectMode {
  if (event.shiftKey) return 'range'
  return event.metaKey || event.ctrlKey ? 'toggle' : 'single'
}

/** Rows as tab-separated text for a spreadsheet: a value with a tab, a line break or a quote is quoted. */
export function rowsAsTsv(rows: (string | null)[][]): string {
  const field = (value: string | null) => {
    const text = value ?? 'NULL'
    return /[\t\n\r"]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text
  }
  return rows.map((row) => row.map(field).join('\t')).join('\n')
}
