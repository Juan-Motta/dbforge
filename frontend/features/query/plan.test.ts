import { describe, expect, it } from 'vitest'
import { messages } from './messages'
import { formatMs, parsePlan as parse } from './plan'

/** Reads a plan with the English texts. */
const parsePlan = (json: unknown) => parse(json, messages.en)

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

  it('writes its warnings and details in the language it is given', () => {
    const spanish = parse(analyzed, messages.es)
    expect(spanish.nodes[1].warnings[0]).toBe(
      'Se estimaron 10 filas pero hubo 480; las estadísticas pueden estar desactualizadas.',
    )
    expect(spanish.nodes[2].detail).toBe('customers · usando customers_pkey · Sort Key: a, b')
  })

  it('keeps every scalar and list field for the detail pane', () => {
    expect(plan.nodes[2].fields).toContainEqual(['Index Name', 'customers_pkey'])
    expect(plan.nodes[2].fields).toContainEqual(['Sort Key', 'a, b'])
  })

  it('does not add up the time of workers that ran in parallel', () => {
    // From a real plan: a parallel Seq Scan in 3 processes (leader + 2 workers),
    // 232.5 ms each, inside a 252 ms query. Summing the loops gave 697 ms.
    const parallel = parsePlan([
      {
        Plan: {
          'Node Type': 'Gather',
          'Total Cost': 29000,
          'Plan Rows': 1,
          'Actual Total Time': 240,
          'Actual Rows': 9,
          'Actual Loops': 1,
          'Workers Launched': 2,
          Plans: [
            {
              'Node Type': 'Seq Scan',
              'Parallel Aware': true,
              'Total Cost': 28204,
              'Plan Rows': 1,
              'Actual Total Time': 232.5,
              'Actual Rows': 3,
              'Actual Loops': 3,
              'Rows Removed by Filter': 323327,
            },
          ],
        },
        'Execution Time': 252.3,
      },
    ])
    const [gather, scan] = parallel.nodes
    expect(scan.selfMs).toBeCloseTo(232.5)
    expect(gather.selfMs).toBeCloseTo(7.5)
    expect(scan.actualRows).toBe(9)
    expect(scan.warnings).toEqual([
      'Reads 969,981 rows only to discard them; an index on the filter may help.',
    ])
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

  it('shares by cost when an analyzed plan ran too fast to time', () => {
    // SQL Server reports whole milliseconds, so small queries measure 0 everywhere.
    const fast = parsePlan([
      {
        Plan: {
          'Node Type': 'Compute Scalar',
          'Total Cost': 100,
          'Actual Total Time': 0,
          'Actual Rows': 3,
          'Actual Loops': 1,
          Plans: [
            {
              'Node Type': 'Clustered Index Scan',
              'Total Cost': 75,
              'Actual Total Time': 0,
              'Actual Rows': 3,
              'Actual Loops': 1,
            },
          ],
        },
      },
    ])
    expect(fast.analyzed).toBe(true)
    expect(fast.nodes.map((n) => [n.actualRows, n.selfMs, n.share])).toEqual([
      [3, 0, 25],
      [3, 0, 75],
    ])
  })
})

describe('MySQL plans', () => {
  it('shows the detail MySQL writes out', () => {
    const plan = parsePlan([
      { Plan: { 'Node Type': 'Filter', Detail: '(o.total > 100)', 'Plan Rows': 1.33 } },
    ])
    expect(plan.nodes[0]).toMatchObject({
      nodeType: 'Filter',
      detail: '(o.total > 100)',
      estimatedRows: 1.33,
    })
    expect(plan.planningMs).toBeNull()
  })

  it('leaves nodes without an estimate unestimated and unwarned', () => {
    const plan = parsePlan([
      { Plan: { 'Node Type': 'Sort', 'Actual Total Time': 5, 'Actual Rows': 50000, 'Actual Loops': 1 } },
    ])
    expect(plan.nodes[0].estimatedRows).toBeNull()
    expect(plan.nodes[0].warnings).toEqual([])
  })
})

describe('formatMs', () => {
  it.each([
    [0.4, '0.40 ms'],
    [24.12, '24.1 ms'],
    [1234, '1.2 s'],
  ])('%d → %s', (ms, text) => expect(formatMs(ms)).toBe(text))
})
