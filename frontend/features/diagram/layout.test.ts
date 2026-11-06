import { describe, expect, it } from 'vitest'
import type { DiagramTable, Relationship, SchemaDiagram } from './api'
import {
  CARD_WIDTH,
  cardHeight,
  defaultSelection,
  fitView,
  layout,
  localRelationships,
  relationshipPath,
  shownColumns,
  zoomAt,
} from './layout'

const rel = (table: string, referencedTable: string, referencedSchema = 'public'): Relationship => ({
  name: `${table}_${referencedTable}_fkey`,
  table,
  columns: [`${referencedTable}_id`],
  referencedSchema,
  referencedTable,
  referencedColumns: ['id'],
})

const table = (name: string): DiagramTable => ({
  name,
  rowEstimate: null,
  columns: [
    { name: 'id', dataType: 'integer', nullable: false, primaryKey: true },
    { name: 'customers_id', dataType: 'integer', nullable: true, primaryKey: false },
    { name: 'note', dataType: 'text', nullable: true, primaryKey: false },
  ],
})

describe('layout', () => {
  const cards = ['order_items', 'orders', 'customers', 'products'].map((name) => ({ name, height: 100 }))
  const positions = layout(cards, [
    rel('orders', 'customers'),
    rel('order_items', 'orders'),
    rel('order_items', 'products'),
  ])
  const columnOf = (name: string) => positions.get(name)!.x / (CARD_WIDTH + 120)

  it('puts referenced tables left of the tables that reference them', () => {
    expect(['customers', 'products', 'orders', 'order_items'].map(columnOf)).toEqual([0, 0, 1, 2])
  })

  it('stacks the tables of a column by name', () => {
    expect(positions.get('customers')!.y).toBe(0)
    expect(positions.get('products')!.y).toBe(100 + 32)
  })

  it('survives cycles and self-references', () => {
    const cyclic = layout(
      ['a', 'b'].map((name) => ({ name, height: 50 })),
      [rel('a', 'b'), rel('b', 'a'), rel('a', 'a')],
    )
    expect(cyclic.size).toBe(2)
  })
})

describe('selection and filtering', () => {
  it('keeps only relationships whose target has a card', () => {
    const diagram: SchemaDiagram = {
      tables: [table('orders'), table('customers')],
      relationships: [rel('orders', 'customers'), rel('orders', 'currencies', 'billing')],
    }
    expect(localRelationships(diagram, 'public').map((r) => r.referencedTable)).toEqual(['customers'])
  })

  it('shows every table of a small schema', () => {
    const diagram: SchemaDiagram = { tables: [table('a'), table('b')], relationships: [] }
    expect(defaultSelection(diagram, [])).toEqual(new Set(['a', 'b']))
  })

  it('starts a big schema with only its related tables', () => {
    const tables = Array.from({ length: 50 }, (_, i) => table(`t${i}`))
    const relationships = [rel('t1', 't2')]
    expect(defaultSelection({ tables, relationships }, relationships)).toEqual(new Set(['t1', 't2']))
  })

  it('can show only key columns', () => {
    const columns = shownColumns(table('orders'), 'keys', new Set(['orders.customers_id']))
    expect(columns.map((c) => c.name)).toEqual(['id', 'customers_id'])
  })
})

describe('relationshipPath', () => {
  it('runs from the left edge of the referencing card to the right edge of a card on its left', () => {
    const path = relationshipPath({ x: 360, y: 0 }, 1, { x: 0, y: 0 }, 0)
    expect(path.startsWith(`M 360 ${32 + 24 + 12}`)).toBe(true)
    expect(path.endsWith(`${CARD_WIDTH} ${32 + 12}`)).toBe(true)
  })

  it('loops a self-reference out of the right edge', () => {
    expect(relationshipPath({ x: 0, y: 0 }, 1, { x: 0, y: 0 }, 0)).toMatch(/^M 240 68 C 290 68/)
  })
})

describe('fitView', () => {
  it('scales down to fit and never zooms in past 100%', () => {
    const cards = [
      { position: { x: 0, y: 0 }, height: cardHeight(3) },
      { position: { x: 2000, y: 0 }, height: cardHeight(3) },
    ]
    expect(fitView(cards, { width: 1000, height: 800 }).scale).toBeCloseTo((1000 - 64) / 2240)
    expect(fitView(cards.slice(0, 1), { width: 1000, height: 800 }).scale).toBe(1)
  })

  it('leaves the top free for the toolbar', () => {
    const view = fitView([{ position: { x: 0, y: 0 }, height: 100 }], { width: 1000, height: 800 })
    expect(view).toEqual({ x: 32, y: 64, scale: 1 })
  })
})

describe('zoomAt', () => {
  it('keeps the point under the cursor in place', () => {
    const view = zoomAt({ x: 100, y: 50, scale: 1 }, 2, { x: 300, y: 250 })
    expect(view).toEqual({ x: -100, y: -150, scale: 2 })
    // The diagram point (200, 200) was at (300, 250) and still is.
    expect(200 * view.scale + view.x).toBe(300)
  })

  it('stays between 10% and 200%', () => {
    expect(zoomAt({ x: 0, y: 0, scale: 1 }, 10, { x: 0, y: 0 }).scale).toBe(2)
    expect(zoomAt({ x: 0, y: 0, scale: 1 }, 0.01, { x: 0, y: 0 }).scale).toBe(0.1)
  })
})
