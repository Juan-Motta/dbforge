import type { DiagramTable, Relationship, SchemaDiagram } from './api'

export const CARD_WIDTH = 240
export const HEADER_HEIGHT = 32
export const ROW_HEIGHT = 24
const GAP_X = 120
const GAP_Y = 32
/** Past this many tables, start with only the related ones so the diagram stays readable. */
const SHOW_ALL_LIMIT = 40

export type ColumnMode = 'all' | 'keys'

export interface Point {
  x: number
  y: number
}

/** Relationships inside the schema; those pointing elsewhere have no card to reach. */
export function localRelationships(diagram: SchemaDiagram, schema: string): Relationship[] {
  const names = new Set(diagram.tables.map((t) => t.name))
  return diagram.relationships.filter((r) => r.referencedSchema === schema && names.has(r.referencedTable))
}

/** Every table when there are few; otherwise only the ones that take part in a relationship. */
export function defaultSelection(diagram: SchemaDiagram, relationships: Relationship[]): Set<string> {
  if (diagram.tables.length <= SHOW_ALL_LIMIT) return new Set(diagram.tables.map((t) => t.name))
  return new Set(relationships.flatMap((r) => [r.table, r.referencedTable]))
}

/** The columns a card shows: all of them, or only primary and foreign keys. */
export function shownColumns(table: DiagramTable, mode: ColumnMode, foreignKeys: Set<string>) {
  if (mode === 'all') return table.columns
  return table.columns.filter((c) => c.primaryKey || foreignKeys.has(`${table.name}.${c.name}`))
}

export function cardHeight(rows: number): number {
  return HEADER_HEIGHT + rows * ROW_HEIGHT + 6
}

/**
 * Places tables in columns by dependency: a table goes one column right of the
 * furthest table it references, so referenced tables sit on the left and lines
 * mostly run right to left. Cycles and self-references are ignored for ranking.
 */
export function layout(
  tables: { name: string; height: number }[],
  relationships: { table: string; referencedTable: string }[],
): Map<string, Point> {
  const names = new Set(tables.map((t) => t.name))
  const references = new Map<string, string[]>()
  for (const r of relationships) {
    if (r.table === r.referencedTable || !names.has(r.table) || !names.has(r.referencedTable)) continue
    references.set(r.table, [...(references.get(r.table) ?? []), r.referencedTable])
  }

  const rank = new Map<string, number>()
  const visiting = new Set<string>()
  const rankOf = (name: string): number => {
    const known = rank.get(name)
    if (known !== undefined) return known
    if (visiting.has(name)) return 0 // a cycle: break it here
    visiting.add(name)
    const value = Math.max(-1, ...(references.get(name) ?? []).map(rankOf)) + 1
    visiting.delete(name)
    rank.set(name, value)
    return value
  }

  const columns = new Map<number, { name: string; height: number }[]>()
  for (const table of [...tables].sort((a, b) => a.name.localeCompare(b.name))) {
    const column = rankOf(table.name)
    columns.set(column, [...(columns.get(column) ?? []), table])
  }

  const positions = new Map<string, Point>()
  for (const [column, members] of columns) {
    let y = 0
    for (const table of members) {
      positions.set(table.name, { x: column * (CARD_WIDTH + GAP_X), y })
      y += table.height + GAP_Y
    }
  }
  return positions
}

/** A curve from the right or left edge of one card's row to the facing edge of another's. */
export function relationshipPath(from: Point, fromRow: number, to: Point, toRow: number): string {
  const y1 = from.y + rowCenter(fromRow)
  const y2 = to.y + rowCenter(toRow)
  if (from.x === to.x && from.y === to.y) {
    // A table referencing itself: loop out of the right edge and back.
    const x = from.x + CARD_WIDTH
    return `M ${x} ${y1} C ${x + 50} ${y1}, ${x + 50} ${y2}, ${x} ${y2}`
  }
  const leftward = to.x + CARD_WIDTH <= from.x
  const x1 = leftward ? from.x : from.x + CARD_WIDTH
  const x2 = leftward ? to.x + CARD_WIDTH : to.x
  const bend = Math.max(40, Math.abs(x2 - x1) / 2) * (leftward ? -1 : 1)
  return `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`
}

function rowCenter(row: number): number {
  return row < 0 ? HEADER_HEIGHT / 2 : HEADER_HEIGHT + row * ROW_HEIGHT + ROW_HEIGHT / 2
}

/** Room kept free around the cards: the toolbar sits on top, the zoom controls at the bottom. */
const FIT_MARGIN = { top: 64, right: 32, bottom: 64, left: 32 }

/** The scale and offset that fit every card into a viewport, never zooming past 100%. */
export function fitView(
  cards: { position: Point; height: number }[],
  viewport: { width: number; height: number },
): View {
  if (cards.length === 0) return { x: FIT_MARGIN.left, y: FIT_MARGIN.top, scale: 1 }
  const left = Math.min(...cards.map((c) => c.position.x))
  const top = Math.min(...cards.map((c) => c.position.y))
  const right = Math.max(...cards.map((c) => c.position.x + CARD_WIDTH))
  const bottom = Math.max(...cards.map((c) => c.position.y + c.height))
  const scale = Math.max(
    0.1,
    Math.min(
      1,
      (viewport.width - FIT_MARGIN.left - FIT_MARGIN.right) / (right - left),
      (viewport.height - FIT_MARGIN.top - FIT_MARGIN.bottom) / (bottom - top),
    ),
  )
  return { x: FIT_MARGIN.left - left * scale, y: FIT_MARGIN.top - top * scale, scale }
}

export interface View {
  x: number
  y: number
  scale: number
}

const MIN_SCALE = 0.1
const MAX_SCALE = 2

/** Zooms by `factor` keeping the diagram point under `anchor` (viewport pixels) in place. */
export function zoomAt(view: View, factor: number, anchor: Point): View {
  const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, view.scale * factor))
  const ratio = scale / view.scale
  return { scale, x: anchor.x - (anchor.x - view.x) * ratio, y: anchor.y - (anchor.y - view.y) * ratio }
}
