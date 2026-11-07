/** One node of a plan, flattened in display order (depth-first). */
export interface PlanNode {
  id: string
  depth: number
  nodeType: string
  /** What the node works on: table, index, join or filter condition, sort key… */
  detail: string
  /** `null` when the engine didn't estimate this node (MySQL sorts, temporary tables). */
  estimatedRows: number | null
  /** Rows produced over all loops; `null` without ANALYZE. */
  actualRows: number | null
  /** Time spent in this node alone, children excluded; `null` without ANALYZE. */
  selfMs: number | null
  /** Planner cost of this node alone, children excluded. */
  selfCost: number
  /** Share of the whole plan: of time when analyzed, else of cost. 0–100. */
  share: number
  warnings: string[]
  /** Every scalar field the server reported, for the detail pane. */
  fields: [string, string][]
}

export interface Plan {
  analyzed: boolean
  planningMs: number | null
  executionMs: number | null
  /** Rows the statement returned; `null` without ANALYZE. */
  rows: number | null
  nodes: PlanNode[]
}

type RawNode = Record<string, unknown> & { Plans?: RawNode[] }

/** Reads the JSON that `EXPLAIN (FORMAT JSON[, ANALYZE])` returns. */
export function parsePlan(json: unknown): Plan {
  const top = (Array.isArray(json) ? json[0] : json) as Record<string, unknown>
  const root = top.Plan as RawNode
  const analyzed = 'Actual Total Time' in root

  const nodes: PlanNode[] = []
  const walk = (raw: RawNode, depth: number, processes: number) => {
    nodes.push(toNode(raw, depth, String(nodes.length), analyzed, processes))
    const childProcesses = parallelProcesses(raw) ?? processes
    raw.Plans?.forEach((child) => walk(child, depth + 1, childProcesses))
  }
  walk(root, 0, 1)

  // Shares follow measured time; a plan too fast to time (SQL Server counts
  // whole milliseconds) shares by estimated cost instead.
  const timed = analyzed && nodes.some((n) => (n.selfMs ?? 0) > 0)
  const weight = (n: PlanNode) => (timed ? (n.selfMs ?? 0) : n.selfCost)
  const total = nodes.reduce((sum, n) => sum + weight(n), 0)
  for (const node of nodes) node.share = total > 0 ? Math.round((weight(node) / total) * 100) : 0

  return {
    analyzed,
    planningMs: numberOrNull(top['Planning Time']),
    executionMs: numberOrNull(top['Execution Time']),
    rows: analyzed ? nodes[0].actualRows : null,
    nodes,
  }
}

/**
 * Below a Gather, each process (the leader and every worker) runs the subtree at
 * the same time, and their loops are added together. Returns how many there were.
 */
function parallelProcesses(raw: RawNode): number | null {
  if (raw['Node Type'] !== 'Gather' && raw['Node Type'] !== 'Gather Merge') return null
  return num(raw['Workers Launched'], num(raw['Workers Planned'])) + 1
}

function toNode(raw: RawNode, depth: number, id: string, analyzed: boolean, processes: number): PlanNode {
  const children = raw.Plans ?? []
  const childProcesses = parallelProcesses(raw) ?? processes
  const loops = num(raw['Actual Loops'], 1)
  // "Actual Total Time" and "Actual Rows" are averages per loop. Loops of parallel
  // processes overlap in time, so their total is split among the processes.
  const totalMs = (node: RawNode, inParallel: number) =>
    (num(node['Actual Total Time']) * num(node['Actual Loops'], 1)) / inParallel
  const estimatedRows = numberOrNull(raw['Plan Rows'])
  const actualPerLoop = num(raw['Actual Rows'])

  const node: PlanNode = {
    id,
    depth,
    nodeType: String(raw['Node Type']),
    detail: describe(raw),
    estimatedRows,
    actualRows: analyzed ? actualPerLoop * loops : null,
    selfMs: analyzed
      ? Math.max(
          0,
          totalMs(raw, processes) - children.reduce((sum, c) => sum + totalMs(c, childProcesses), 0),
        )
      : null,
    selfCost: Math.max(
      0,
      num(raw['Total Cost']) - children.reduce((sum, c) => sum + num(c['Total Cost']), 0),
    ),
    share: 0,
    warnings: [],
    fields: fieldsOf(raw),
  }

  if (analyzed) {
    const [low, high] =
      estimatedRows === null
        ? [0, 0]
        : [Math.min(estimatedRows, actualPerLoop), Math.max(estimatedRows, actualPerLoop)]
    if (estimatedRows !== null && high >= 100 && high >= Math.max(low, 1) * 10) {
      node.warnings.push(
        `Estimated ${fmt(estimatedRows)} rows but got ${fmt(actualPerLoop)}; statistics may be stale.`,
      )
    }
    const removed = num(raw['Rows Removed by Filter']) * loops
    if (node.nodeType === 'Seq Scan' && removed > 1000 && removed > (node.actualRows ?? 0) * 10) {
      node.warnings.push(`Reads ${fmt(removed)} rows only to discard them; an index on the filter may help.`)
    }
  }
  return node
}

/** Scalar fields first, then lists joined; child plans are shown as their own nodes. */
function fieldsOf(raw: RawNode): [string, string][] {
  const entries = Object.entries(raw).filter(([key, value]) => key !== 'Plans' && value !== null)
  const scalars = entries.filter(([, value]) => typeof value !== 'object')
  const lists = entries.filter(([, value]) => Array.isArray(value))
  return [
    ...scalars.map(([key, value]): [string, string] => [key, String(value)]),
    ...lists.map(([key, value]): [string, string] => [key, (value as unknown[]).join(', ')]),
  ]
}

/** The parts of a node worth a glance, joined: `orders o · Filter: (total > 100)`. */
function describe(raw: RawNode): string {
  const parts: string[] = []
  // MySQL plans come with their description already written out.
  if (typeof raw['Detail'] === 'string') parts.push(raw['Detail'])
  const relation = raw['Relation Name']
  if (typeof relation === 'string') {
    const alias = raw['Alias']
    parts.push(alias && alias !== relation ? `${relation} ${alias}` : relation)
  }
  if (typeof raw['Index Name'] === 'string') parts.push(`using ${raw['Index Name']}`)
  for (const key of ['Hash Cond', 'Merge Cond', 'Join Filter', 'Index Cond', 'Recheck Cond', 'Filter']) {
    if (typeof raw[key] === 'string') parts.push(`${key}: ${raw[key]}`)
  }
  for (const key of ['Sort Key', 'Group Key']) {
    if (Array.isArray(raw[key])) parts.push(`${key}: ${(raw[key] as string[]).join(', ')}`)
  }
  return parts.join(' · ')
}

function num(value: unknown, fallback = 0): number {
  return typeof value === 'number' ? value : fallback
}

function numberOrNull(value: unknown): number | null {
  return typeof value === 'number' ? value : null
}

function fmt(n: number): string {
  return Math.round(n).toLocaleString('en-US')
}

/** 0.4 ms, 24.1 ms, 1.2 s */
export function formatMs(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)} s`
  return `${ms < 10 ? ms.toFixed(2) : ms.toFixed(1)} ms`
}
