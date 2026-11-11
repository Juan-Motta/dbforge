import { describe, expect, it, vi } from 'vitest'
import type { MenuItem } from '../components/ContextMenu'
import { NO_SSH_TUNNEL, type Connection } from '../domain/connection'
import { messages as adminMessages } from '../features/admin/messages'
import type { TreeNode } from '../features/explorer/tree'
import { explorerMenu, type MenuActions } from './explorerMenu'
import { messages } from './messages'

const postgres: Connection = {
  id: 'pg',
  name: 'pg',
  engine: 'postgres',
  environment: null,
  host: 'h',
  port: 5432,
  database: 'shop',
  username: 'app',
  ssl: { mode: 'prefer', caFile: '', certFile: '', keyFile: '' },
  ssh: NO_SSH_TUNNEL,
  srv: false,
  agentAccess: 'none',
}

function node(kind: TreeNode['kind'], path: string[], connection = postgres): TreeNode {
  const base = {
    id: path.join('/'),
    name: path.at(-1) ?? connection.name,
    connectionId: connection.id,
    path,
    expandable: true,
  }
  return kind === 'connection' ? { ...base, kind, connection } : { ...base, kind }
}

function actions(): MenuActions {
  return {
    editConnection: vi.fn(),
    duplicateConnection: vi.fn(),
    deleteConnection: vi.fn(),
    newObject: vi.fn(),
    newQuery: vi.fn(),
    openTable: vi.fn(),
    openDiagram: vi.fn(),
    exportTable: vi.fn(),
    importRows: vi.fn(),
    dump: vi.fn(),
    dropDatabase: vi.fn(),
    copy: vi.fn(),
    structure: vi.fn(async () => ({
      columns: [{ name: 'id' }, { name: 'name' }],
      ddl: 'CREATE TABLE users (…);',
    })) as unknown as MenuActions['structure'],
    refresh: vi.fn(),
  }
}

const labels = (items: MenuItem[]): (string | [string, string[]])[] =>
  items.map((item) => (item.items ? [item.label, item.items.map((sub) => sub.label)] : item.label))

const menu = (n: TreeNode, a: MenuActions, connection = postgres, m = messages.en) =>
  explorerMenu(n, connection, a, m, m === messages.en ? adminMessages.en : adminMessages.es)

const pick = (items: MenuItem[], ...path: string[]): MenuItem => {
  const [first, ...rest] = path
  const item = items.find((i) => i.label === first)!
  return rest.length ? pick(item.items!, ...rest) : item
}

describe('explorerMenu', () => {
  it('offers each node what it can do', () => {
    const a = actions()
    expect(labels(menu(node('connection', []), a))).toEqual([
      'New query',
      ['New', ['Database', 'Role', 'User']],
      'Refresh',
      'Copy connection URL',
      'Duplicate connection',
      'Edit connection',
      'Delete connection',
    ])
    expect(labels(menu(node('database', ['shop']), a))).toEqual([
      'New query',
      'Refresh',
      'Export database…',
      'Import SQL file…',
      'Drop database…',
    ])
    expect(labels(menu(node('schema', ['shop', 'public']), a))).toEqual([
      'New query',
      'Show ER diagram',
      'Refresh',
    ])
    expect(labels(menu(node('table', ['shop', 'public', 'users']), a))).toEqual([
      'Open data',
      'View structure',
      'New query',
      'Export…',
      'Import rows…',
      ['Copy', ['Name', 'SELECT statement', 'INSERT statement', 'CREATE statement']],
    ])
    // Rows can't be imported into a view.
    expect(labels(menu(node('view', ['shop', 'public', 'v']), a))).not.toContain('Import rows…')
    expect(labels(menu(node('view', ['shop', 'public', 'v']), a)).at(-1)).toEqual([
      'Copy',
      ['Name', 'SELECT statement', 'INSERT statement', 'Definition'],
    ])
  })

  it('leaves out what an engine lacks', () => {
    const a = actions()
    const redis = { ...postgres, engine: 'redis' as const }
    expect(labels(menu(node('database', ['db0'], redis), a, redis))).toEqual(['Refresh'])
    const mongo = { ...postgres, engine: 'mongodb' as const }
    expect(labels(menu(node('table', ['shop', 'orders'], mongo), a, mongo)).at(-1)).toEqual([
      'Copy',
      ['Name', 'find() query', 'countDocuments() query'],
    ])
    const mysql = { ...postgres, engine: 'mysql' as const }
    expect(labels(menu(node('database', ['shop'], mysql), a, mysql))).toEqual([
      'New query',
      'Show ER diagram',
      'Refresh',
      'Drop database…',
    ])
  })

  it('opens and copies what the node is', async () => {
    const a = actions()
    const users = node('table', ['shop', 'public', 'users'])
    const items = menu(users, a)
    pick(items, 'View structure').onSelect!()
    expect(a.openTable).toHaveBeenCalledWith(
      postgres,
      { database: 'shop', schema: 'public', name: 'users' },
      'columns',
    )
    pick(items, 'New query').onSelect!()
    expect(a.newQuery).toHaveBeenCalledWith(
      postgres,
      'shop',
      'SELECT * FROM "public"."users" LIMIT 100;',
      undefined,
    )
    pick(items, 'Copy', 'INSERT statement').onSelect!()
    await expect(vi.mocked(a.copy).mock.calls[0][0]).resolves.toBe(
      'INSERT INTO "public"."users" ("id", "name")\nVALUES (NULL, NULL);',
    )

    const cosmos = { ...postgres, engine: 'cosmosdb' as const }
    pick(menu(node('table', ['shop', 'orders'], cosmos), a, cosmos), 'New query').onSelect!()
    expect(a.newQuery).toHaveBeenLastCalledWith(cosmos, 'shop', 'SELECT TOP 100 * FROM c', 'orders')

    pick(menu(node('database', ['shop']), a), 'Drop database…').onSelect!()
    expect(a.dropDatabase).toHaveBeenCalledWith(postgres, 'shop')

    pick(menu(node('connection', []), a), 'Copy connection URL').onSelect!()
    expect(a.copy).toHaveBeenLastCalledWith('postgresql://app@h:5432/shop')
  })

  it('writes the menu in Spanish', () => {
    expect(labels(menu(node('schema', ['shop', 'public']), actions(), postgres, messages.es))).toEqual([
      'Nueva consulta',
      'Mostrar diagrama ER',
      'Actualizar',
    ])
  })
})
