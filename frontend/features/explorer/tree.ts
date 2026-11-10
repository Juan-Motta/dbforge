import type { Connection } from '../../domain/connection'
import type { DbObject } from './api'
import type { messages } from './messages'

type Messages = typeof messages.en

export type ObjectKind = 'database' | 'schema' | 'folder' | 'table' | 'view'

interface BaseNode {
  id: string
  name: string
  /** Short secondary text shown on the right, e.g. a row count. */
  meta?: string
  connectionId: string
  /** Path sent to the backend to list this node's children. */
  path: string[]
  expandable: boolean
  /** `undefined` until loaded. */
  children?: TreeNode[]
  loading?: boolean
  error?: string
}

export type TreeNode =
  (BaseNode & { kind: 'connection'; connection: Connection }) | (BaseNode & { kind: ObjectKind })

export type TreeRow =
  | { type: 'node'; node: TreeNode; depth: number; expanded: boolean }
  | { type: 'message'; id: string; depth: number; text: string; tone: 'muted' | 'error' }

// Joins path segments into ids; object names cannot contain this control character.
const SEP = '\u001f'

export function connectionNode(connection: Connection): TreeNode {
  return {
    id: connection.id,
    name: connection.name,
    kind: 'connection',
    connection,
    connectionId: connection.id,
    path: [],
    expandable: true,
  }
}

const compact = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 })

/**
 * Turns the objects listed under `parent` into tree nodes. Tables and views are
 * grouped into "Tables" and "Views" folders, named in the language of `m`, which
 * arrive already loaded.
 */
export function toChildNodes(parent: TreeNode, objects: DbObject[], m: Messages): TreeNode[] {
  // A Redis database holds keys, not schemas: it opens in the key browser.
  const keyspaces = parent.kind === 'connection' && parent.connection.engine === 'redis'
  const toNode = (object: DbObject): TreeNode => ({
    id: parent.id + SEP + object.name,
    name: object.name,
    kind: object.kind,
    meta: object.rowEstimate === null ? undefined : compact.format(object.rowEstimate).toLowerCase(),
    connectionId: parent.connectionId,
    path: [...parent.path, object.name],
    expandable: (object.kind === 'database' && !keyspaces) || object.kind === 'schema',
  })

  // `key` names the folder in its id, the same in every language, so it stays expanded.
  const folder = (key: string, name: string, kind: DbObject['kind']): TreeNode[] => {
    const items = objects.filter((o) => o.kind === kind).map(toNode)
    if (items.length === 0) return []
    return [
      {
        id: parent.id + SEP + '#' + key,
        name,
        kind: 'folder',
        meta: String(items.length),
        connectionId: parent.connectionId,
        path: parent.path,
        expandable: true,
        children: items,
      },
    ]
  }

  const others = objects.filter((o) => o.kind !== 'table' && o.kind !== 'view').map(toNode)
  return [...others, ...folder('Tables', m.tables, 'table'), ...folder('Views', m.views, 'view')]
}

/** Flattens the tree into the rows currently visible on screen. */
export function visibleRows(
  nodes: TreeNode[],
  isExpanded: (id: string) => boolean,
  m: Messages,
  depth = 0,
): TreeRow[] {
  return nodes.flatMap((node): TreeRow[] => {
    const hasContent = node.children !== undefined || node.loading || node.error !== undefined
    const expanded = node.expandable && isExpanded(node.id) && hasContent
    const row: TreeRow = { type: 'node', node, depth, expanded }
    if (!expanded) return [row]

    const message = (text: string, tone: 'muted' | 'error'): TreeRow[] => [
      row,
      { type: 'message', id: node.id + SEP + '#message', depth: depth + 1, text, tone },
    ]
    if (node.loading) return message(m.loading, 'muted')
    if (node.error !== undefined) return message(node.error, 'error')
    if (node.children!.length === 0) return message(m.noObjects, 'muted')
    return [row, ...visibleRows(node.children!, isExpanded, m, depth + 1)]
  })
}

/**
 * Keeps nodes whose name contains `query` (case-insensitive), plus the
 * ancestors needed to reach them. A matching node keeps all its children.
 * Only loaded nodes are searched.
 */
export function filterTree(nodes: TreeNode[], query: string): TreeNode[] {
  const needle = query.trim().toLowerCase()
  if (!needle) return nodes

  return nodes.flatMap((node) => {
    if (node.name.toLowerCase().includes(needle)) return [node]
    const children = filterTree(node.children ?? [], needle)
    return children.length > 0 ? [{ ...node, children }] : []
  })
}
