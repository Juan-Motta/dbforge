import { describe, expect, it } from 'vitest'
import { formatMs, parsePlan } from './plan'

// Shaped like PostgreSQL's EXPLAIN (ANALYZE, FORMAT JSON) output.
const analyzed = [
  {
    Plan: {
      'Node Type': 'Hash Join',
      'Total Cost': 300,
      'Plan Rows': 500,
      'Actual Total Time': 30,
      'Actual Rows': 480,
      'Actual Loops': 1,
      'Hash Cond': '(o.customer_id = c.id)',
      Plans: [
        {
          'Node Type': 'Seq Scan',
          'Relation Name': 'orders',
          Alias: 'o',
          'Total Cost': 200,
          'Plan Rows': 10,
          'Actual Total Time': 24,
          'Actual Rows': 480,
          'Actual Loops': 1,
          Filter: '(total > 100)',
          'Rows Removed by Filter': 50000,
        },
        {
          'Node Type': 'Index Scan',
          'Relation Name': 'customers',
          Alias: 'customers',
          'Index Name': 'customers_pkey',
          'Total Cost': 40,
          'Plan Rows': 1,
          'Actual Total Time': 0.002,
          'Actual Rows': 1,
          'Actual Loops': 480,
          'Sort Key': ['a', 'b'],
        },
      ],
    },
    'Planning Time': 0.9,
    'Execution Time': 30.5,
  },
]

describe('parsePlan', () => {
  const plan = parsePlan(analyzed)

  it('flattens the tree depth-first', () => {
    expect(plan.nodes.map((n) => [n.depth, n.nodeType])).toEqual([
      [0, 'Hash Join'],
      [1, 'Seq Scan'],
      [1, 'Index Scan'],
    ])
  })

  it('reads timings and the rows returned', () => {
    expect(plan).toMatchObject({ analyzed: true, planningMs: 0.9, executionMs: 30.5, rows: 480 })
  })

  it('counts rows and time over every loop, without the children', () => {
    const [join, scan, index] = plan.nodes
    expect(index.actualRows).toBe(480)
    expect(index.selfMs).toBeCloseTo(0.96)
    expect(scan.selfMs).toBe(24)
    expect(join.selfMs).toBeCloseTo(30 - 24 - 0.96)
  })

  it('gives each node its share of the total', () => {
    expect(plan.nodes.map((n) => n.share)).toEqual([17, 80, 3])
  })

  it('describes what each node works on', () => {
    expect(plan.nodes.map((n) => n.detail)).toEqual([
      'Hash Cond: (o.customer_id = c.id)',
      'orders o · Filter: (total > 100)',
      'customers · using customers_pkey · Sort Key: a, b',
    ])
  })

  it('warns about bad estimates and wasteful scans', () => {
    expect(plan.nodes[1].warnings).toEqual([
      'Estimated 10 rows but got 480; statistics may be stale.',
      'Reads 50,000 rows only to discard them; an index on the filter may help.',
    ])
    expect(plan.nodes[0].warnings).toEqual([])
  })

  it('keeps every scalar and list field for the detail pane', () => {
    expect(plan.nodes[2].fields).toContainEqual(['Index Name', 'customers_pkey'])
    expect(plan.nodes[2].fields).toContainEqual(['Sort Key', 'a, b'])
  })

  it('falls back to cost shares without ANALYZE', () => {
    const estimate = parsePlan([
      {
        Plan: {
          'Node Type': 'Limit',
          'Total Cost': 100,
          'Plan Rows': 10,
          Plans: [{ 'Node Type': 'Seq Scan', 'Total Cost': 75, 'Plan Rows': 10 }],
        },
      },
    ])
    expect(estimate).toMatchObject({ analyzed: false, executionMs: null, rows: null })
    expect(estimate.nodes.map((n) => [n.actualRows, n.selfMs, n.share])).toEqual([
      [null, null, 25],
      [null, null, 75],
    ])
  })
})

describe('formatMs', () => {
  it.each([
    [0.4, '0.40 ms'],
    [24.12, '24.1 ms'],
    [1234, '1.2 s'],
  ])('%d → %s', (ms, text) => expect(formatMs(ms)).toBe(text))
})
