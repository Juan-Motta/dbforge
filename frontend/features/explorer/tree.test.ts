import { describe, expect, it } from 'vitest'
import type { Connection } from '../../domain/connection'
import type { DbObject } from './api'
import { messages } from './messages'
import {
  connectionNode,
  filterTree,
  toChildNodes as toNodes,
  visibleRows as rowsOf,
  type TreeNode,
  type TreeRow,
} from './tree'

// The English texts, unless a test asks for another language.
const toChildNodes = (parent: TreeNode, objects: DbObject[]) => toNodes(parent, objects, messages.en)
const visibleRows = (nodes: TreeNode[], isExpanded: (id: string) => boolean) =>
  rowsOf(nodes, isExpanded, messages.en)

const connection: Connection = {
  id: 'pg',
  name: 'local',
  engine: 'postgres',
  environment: null,
  host: 'localhost',
  port: 5432,
  database: 'shop',
  username: 'app',
  tls: false,
  allowSelfSigned: false,
  srv: false,
  agentAccess: 'none',
}

const object = (name: string, kind: DbObject['kind'], rowEstimate: number | null = null): DbObject => ({
  name,
  kind,
  rowEstimate,
})

/** connection → shop → public → Tables(customers, orders) + Views(active) */
function sampleTree(): TreeNode[] {
  const root = connectionNode(connection)
  const [shop] = toChildNodes(root, [object('shop', 'database')])
  const [pub] = toChildNodes(shop, [object('public', 'schema')])
  pub.children = toChildNodes(pub, [
    object('customers', 'table', 48_210),
    object('orders', 'table', 612),
    object('active', 'view'),
  ])
  shop.children = [pub]
  root.children = [shop]
  return [root]
}

const labels = (rows: TreeRow[]) =>
  rows.map((r) => (r.type === 'node' ? `${r.depth}:${r.node.name}` : `${r.depth}:(${r.text})`))

describe('toChildNodes', () => {
  it('builds paths and ids from the parent', () => {
    const [shop] = toChildNodes(connectionNode(connection), [object('shop', 'database')])
    expect(shop).toMatchObject({ connectionId: 'pg', path: ['shop'], expandable: true })
    expect(shop.id).not.toBe(connection.id)
  })

  it('groups tables and views into loaded folders with counts and compact row estimates', () => {
    const [pub] = sampleTree()[0].children![0].children!
    const [tables, views] = pub.children!
    expect(tables).toMatchObject({ kind: 'folder', name: 'Tables', meta: '2', path: ['shop', 'public'] })
    expect(tables.children!.map((t) => [t.name, t.meta, t.expandable])).toEqual([
      ['customers', '48.2k', false],
      ['orders', '612', false],
    ])
    expect(views).toMatchObject({ name: 'Views', meta: '1' })
  })

  it('omits empty folders', () => {
    const nodes = toChildNodes(connectionNode(connection), [object('t', 'table')])
    expect(nodes.map((n) => n.name)).toEqual(['Tables'])
  })

  it('names the folders in the language it is given, under the same ids', () => {
    const objects = [object('t', 'table'), object('v', 'view')]
    const english = toChildNodes(connectionNode(connection), objects)
    const spanish = toNodes(connectionNode(connection), objects, messages.es)
    expect(spanish.map((n) => n.name)).toEqual(['Tablas', 'Vistas'])
    expect(spanish.map((n) => n.id)).toEqual(english.map((n) => n.id))
  })
})

describe('Redis databases', () => {
  it('open in the key browser instead of expanding', () => {
    const redis = connectionNode({ ...connection, id: 'cache', engine: 'redis', port: 6379 })
    const [db0] = toChildNodes(redis, [object('db0', 'database', 12)])
    expect([db0.expandable, db0.meta, db0.path]).toEqual([false, '12', ['db0']])
    const [shop] = toChildNodes(connectionNode(connection), [object('shop', 'database')])
    expect(shop.expandable).toBe(true)
  })
})

describe('visibleRows', () => {
  it('shows only roots when nothing is expanded', () => {
    expect(labels(visibleRows(sampleTree(), () => false))).toEqual(['0:local'])
  })

  it('shows children of expanded, loaded nodes', () => {
    const rows = visibleRows(sampleTree(), () => true)
    expect(labels(rows)).toEqual([
      '0:local',
      '1:shop',
      '2:public',
      '3:Tables',
      '4:customers',
      '4:orders',
      '3:Views',
      '4:active',
    ])
  })

  it('shows loading, error and empty states under an expanded node', () => {
    const root = connectionNode(connection)
    expect(labels(visibleRows([{ ...root, loading: true }], () => true))).toEqual(['0:local', '1:(Loading…)'])
    expect(labels(visibleRows([{ ...root, error: 'refused' }], () => true))).toEqual([
      '0:local',
      '1:(refused)',
    ])
    expect(labels(visibleRows([{ ...root, children: [] }], () => true))).toEqual([
      '0:local',
      '1:(No objects)',
    ])
  })

  it('treats an expanded node with nothing loaded as collapsed', () => {
    const [row] = visibleRows([connectionNode(connection)], () => true)
    expect(row).toMatchObject({ type: 'node', expanded: false })
  })
})

describe('filterTree', () => {
  it('returns the same tree for an empty query', () => {
    const tree = sampleTree()
    expect(filterTree(tree, '  ')).toBe(tree)
  })

  it('keeps matches and the ancestors that lead to them', () => {
    const rows = visibleRows(filterTree(sampleTree(), 'CUST'), () => true)
    expect(labels(rows)).toEqual(['0:local', '1:shop', '2:public', '3:Tables', '4:customers'])
  })

  it('keeps all children of a matching node', () => {
    const rows = visibleRows(filterTree(sampleTree(), 'views'), () => true)
    expect(labels(rows)).toEqual(['0:local', '1:shop', '2:public', '3:Views', '4:active'])
  })

  it('returns nothing when no node matches', () => {
    expect(filterTree(sampleTree(), 'zzz')).toEqual([])
  })
})
