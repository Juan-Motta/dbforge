import type { MenuItem } from '../components/ContextMenu'
import {
  defaultDatabase,
  defaultSchema,
  DUMP_ENGINES,
  hasDiagram,
  NEW_OBJECTS,
  RELATIONAL_ENGINES,
  sqlDialectOf,
  type Connection,
  type NewObjectKind,
} from '../domain/connection'
import { tableRefFromPath, type TableRef } from '../domain/table'
import { MENU_ICONS, menuLabelOf } from '../features/admin/newObject'
import type { AdminMessages } from '../features/admin/messages'
import { connectionUrl } from '../features/connections/url'
import type { TreeNode } from '../features/explorer/tree'
import { insertStatement, previewQuery, selectStatement } from '../features/explorer/snippets'
import type { TableStructure } from '../features/table-structure/api'
import type { AppMessages } from './messages'
import type { TableTabView } from './tabs'

/** What the explorer's menus do; `App` carries them out. */
export interface MenuActions {
  editConnection: (connection: Connection) => void
  duplicateConnection: (connection: Connection) => void
  deleteConnection: (connection: Connection) => void
  newObject: (connection: Connection, kind: NewObjectKind) => void
  newQuery: (connection: Connection, database?: string, sql?: string, container?: string) => void
  openTable: (connection: Connection, table: TableRef, view: TableTabView) => void
  openDiagram: (connection: Connection, database: string, schema: string) => void
  exportTable: (connection: Connection, table: TableRef) => void
  dump: (mode: 'export' | 'import', connection: Connection, database: string) => void
  /** Puts the text on the clipboard and says so; call it from the click itself. */
  copy: (text: string | Promise<string>) => void
  structure: (connection: Connection, table: TableRef) => Promise<TableStructure>
  refresh: () => void
}

/**
 * The database and schema a node's ER diagram shows: a schema, or a
 * database that is its own schema (MySQL); `null` when it has none.
 */
export function diagramTarget(
  node: TreeNode,
  connection: Connection,
): { database: string; schema: string } | null {
  if (!hasDiagram(connection.engine)) return null
  const [database, schemaName] = node.path
  if (node.kind === 'schema') return { database, schema: schemaName }
  if (node.kind === 'database' && defaultSchema(connection.engine, database) === database) {
    return { database, schema: database }
  }
  return null
}

/** A node's right-click menu. */
export function explorerMenu(
  node: TreeNode,
  connection: Connection,
  actions: MenuActions,
  m: AppMessages,
  adminM: AdminMessages,
): MenuItem[] {
  const refresh: MenuItem = { label: m.refresh, icon: 'refresh', onSelect: actions.refresh }
  const diagram = diagramTarget(node, connection)
  const diagramItem: MenuItem[] = diagram
    ? [
        {
          label: m.showDiagram,
          icon: 'diagram',
          onSelect: () => actions.openDiagram(connection, diagram.database, diagram.schema),
        },
      ]
    : []

  switch (node.kind) {
    case 'connection': {
      const create = NEW_OBJECTS[connection.engine].map((kind) => ({
        label: menuLabelOf(kind, adminM),
        icon: MENU_ICONS[kind],
        onSelect: () => actions.newObject(connection, kind),
      }))
      return [
        {
          label: m.newQuery,
          icon: 'code',
          onSelect: () => actions.newQuery(connection, defaultDatabase(connection) || undefined),
        },
        ...(create.length > 0 ? [{ label: m.newMenu, icon: 'plus' as const, items: create }] : []),
        refresh,
        {
          label: m.copyConnectionUrl,
          icon: 'external',
          onSelect: () => actions.copy(connectionUrl(connection)),
        },
        {
          label: m.duplicateConnection,
          icon: 'duplicate',
          onSelect: () => actions.duplicateConnection(connection),
        },
        { label: m.editConnection, icon: 'edit', onSelect: () => actions.editConnection(connection) },
        { label: m.deleteConnection, icon: 'trash', onSelect: () => actions.deleteConnection(connection) },
      ]
    }
    case 'database': {
      const dump: MenuItem[] = DUMP_ENGINES.includes(connection.engine)
        ? [
            {
              label: m.exportDatabase,
              icon: 'download',
              onSelect: () => actions.dump('export', connection, node.name),
            },
            { label: m.importSqlFile, onSelect: () => actions.dump('import', connection, node.name) },
          ]
        : []
      // A Redis database is a keyspace, opened by double-clicking it; it has no console of its own.
      const query: MenuItem[] =
        connection.engine === 'redis'
          ? []
          : [{ label: m.newQuery, icon: 'code', onSelect: () => actions.newQuery(connection, node.name) }]
      return [...query, ...diagramItem, refresh, ...dump]
    }
    case 'schema':
      return [
        { label: m.newQuery, icon: 'code', onSelect: () => actions.newQuery(connection, node.path[0]) },
        ...diagramItem,
        refresh,
      ]
    case 'table':
    case 'view':
      return tableMenu(node, connection, actions, m)
    case 'folder':
      return [refresh]
  }
}

function tableMenu(node: TreeNode, connection: Connection, actions: MenuActions, m: AppMessages): MenuItem[] {
  const table = tableRefFromPath(node.path)
  const dialect = sqlDialectOf(connection.engine)
  const cosmos = connection.engine === 'cosmosdb'
  const relational = RELATIONAL_ENGINES.includes(connection.engine)
  const columns = () => actions.structure(connection, table).then((s) => s.columns.map((c) => c.name))

  const copy: MenuItem[] = [
    { label: m.copyName, onSelect: () => actions.copy(table.name) },
    ...(relational
      ? [
          {
            label: m.copySelect,
            onSelect: () => actions.copy(columns().then((names) => selectStatement(dialect, table, names))),
          },
          {
            label: m.copyInsert,
            onSelect: () => actions.copy(columns().then((names) => insertStatement(dialect, table, names))),
          },
          {
            label: node.kind === 'view' ? m.copyDefinition : m.copyCreate,
            onSelect: () => actions.copy(actions.structure(connection, table).then((s) => s.ddl)),
          },
        ]
      : []),
  ]

  return [
    { label: m.openData, icon: 'table', onSelect: () => actions.openTable(connection, table, 'data') },
    {
      label: m.viewStructure,
      icon: 'column',
      onSelect: () => actions.openTable(connection, table, 'columns'),
    },
    {
      label: m.newQuery,
      icon: 'code',
      onSelect: () =>
        actions.newQuery(
          connection,
          table.database,
          previewQuery(dialect, table),
          cosmos ? table.name : undefined,
        ),
    },
    { label: m.exportMenu, icon: 'download', onSelect: () => actions.exportTable(connection, table) },
    { label: m.copyMenu, items: copy },
  ]
}
