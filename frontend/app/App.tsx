import { useEffect, useState } from 'react'
import {
  defaultDatabase,
  defaultSchema,
  DUMP_ENGINES,
  hasDiagram,
  NEW_OBJECTS,
  type Connection,
  type NewObjectKind,
} from '../domain/connection'
import { tableRefFromPath } from '../domain/table'
import { NewObjectDialog } from '../features/admin/NewObjectDialog'
import { messages as adminMessages } from '../features/admin/messages'
import { MENU_ICONS, menuLabelOf } from '../features/admin/newObject'
import { ConnectionDialog } from '../features/connections/ConnectionDialog'
import { DeleteConnectionDialog } from '../features/connections/DeleteConnectionDialog'
import { ExportDatabaseDialog, ImportDatabaseDialog } from '../features/dump/DumpDialogs'
import { useConnections } from '../features/connections/useConnections'
import type { MenuItem } from '../components/ContextMenu'
import { ResizeHandle } from '../components/ResizeHandle'
import { usePanelWidth } from '../components/usePanelWidth'
import { Explorer } from '../features/explorer/Explorer'
import type { TreeNode } from '../features/explorer/tree'
import { CommandPalette, type Command } from '../features/palette/CommandPalette'
import { SettingsDialog } from '../features/settings/SettingsDialog'
import { useTheme } from '../features/settings/useTheme'
import { TitleBar } from '../features/shell/TitleBar'
import {
  closeConnectionTabs,
  closeTab,
  newQueryTab,
  NO_TABS,
  openDiagramTab,
  openKeyspaceTab,
  openTableTab,
} from './tabs'
import { useMessages } from '../lib/i18n/useLanguage'
import { messages } from './messages'
import { Workspace } from './Workspace'
import styles from './App.module.css'

/** `null` while closed; `{}` for a new connection. */
type DialogState = { connection?: Connection } | null

interface Selection {
  connectionId: string
  path: string[]
}

export function App() {
  const { connections, loadError, save, remove } = useConnections()
  // Hidden with ⌘B for more room; it stays mounted so its expanded nodes survive.
  const [explorerOpen, setExplorerOpen] = useState(true)
  const [explorerWidth, setExplorerWidth, resetExplorerWidth] = usePanelWidth(
    'dbforge.explorerWidth',
    280,
    200,
    560,
  )
  const [selection, setSelection] = useState<Selection | null>(null)
  const [dialog, setDialog] = useState<DialogState>(null)
  // A database being exported with pg_dump, or a file being imported into one with psql.
  const [dump, setDump] = useState<{
    mode: 'export' | 'import'
    connection: Connection
    database: string
    refresh: () => void
  } | null>(null)
  // A database, role or user being created from a connection's menu.
  const [creating, setCreating] = useState<{
    connection: Connection
    kind: NewObjectKind
    refresh: () => void
  } | null>(null)
  const [deleting, setDeleting] = useState<Connection | null>(null)
  const [tabs, setTabs] = useState(NO_TABS)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const theme = useTheme()
  const m = useMessages(messages)
  const adminM = useMessages(adminMessages)

  const activeConnection = connections.find((c) => c.id === selection?.connectionId)
  const activePath = activeConnection ? (selection?.path ?? []) : []

  // A new query runs against the selected connection and database. Redis has
  // no query consoles: its key browser holds the CLI.
  const newQuery = activeConnection
    ? activeConnection.engine === 'redis'
      ? () =>
          setTabs((state) =>
            openKeyspaceTab(
              state,
              activeConnection.id,
              activeConnection.name,
              activePath[0] ?? `db${activeConnection.database || 0}`,
            ),
          )
      : activeConnection.engine === 'cosmosdb'
        ? // A Cosmos DB console queries one container: the selected one, if any.
          () =>
            setTabs((state) =>
              newQueryTab(
                state,
                m,
                activeConnection.id,
                activePath[0] ?? (activeConnection.database || undefined),
                undefined,
                activePath[1],
              ),
            )
        : () => setTabs((state) => newQueryTab(state, m, activeConnection.id, activePath[0]))
    : undefined

  // The diagram of the selected schema, or the default one when only a connection or database is selected.
  const diagramDatabase = activePath[0] ?? (activeConnection && defaultDatabase(activeConnection))
  const diagramSchema =
    activeConnection && diagramDatabase
      ? (activePath[1] ?? defaultSchema(activeConnection.engine, diagramDatabase))
      : undefined
  const openDiagram =
    activeConnection && diagramDatabase && hasDiagram(activeConnection.engine)
      ? () =>
          setTabs((state) => openDiagramTab(state, m, activeConnection.id, diagramDatabase, diagramSchema!))
      : undefined

  // ⌘K / Ctrl+K toggles the palette from anywhere, the SQL editor included.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setPaletteOpen((open) => !open)
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'b') {
        event.preventDefault()
        setExplorerOpen((open) => !open)
      }
    }
    window.addEventListener('keydown', onKeyDown, { capture: true })
    return () => window.removeEventListener('keydown', onKeyDown, { capture: true })
  }, [])

  const commands: Command[] = [
    ...(newQuery
      ? [{ id: 'new-query', label: m.newQuery, detail: activeConnection?.name, run: newQuery }]
      : []),
    ...(openDiagram
      ? [
          {
            id: 'diagram',
            label: m.showDiagram,
            detail:
              diagramDatabase === diagramSchema ? diagramDatabase : `${diagramDatabase}.${diagramSchema}`,
            run: openDiagram,
          },
        ]
      : []),
    { id: 'new-connection', label: m.newConnection, run: () => setDialog({}) },
    { id: 'theme-dark', label: m.themeDark, run: () => theme.choose('dark') },
    { id: 'theme-light', label: m.themeLight, run: () => theme.choose('light') },
    { id: 'theme-system', label: m.themeSystem, run: () => theme.choose('system') },
    { id: 'settings', label: m.openSettings, run: () => setSettingsOpen(true) },
  ]

  /** A tree node's right-click menu: a connection's actions, or a database's export and import. */
  const menuOf = (node: TreeNode, refresh: () => void): MenuItem[] => {
    if (node.kind === 'connection') {
      const connection = node.connection
      const create = NEW_OBJECTS[connection.engine].map((kind) => ({
        label: menuLabelOf(kind, adminM),
        icon: MENU_ICONS[kind],
        onSelect: () => setCreating({ connection, kind, refresh }),
      }))
      return [
        { label: m.editConnection, icon: 'edit', onSelect: () => setDialog({ connection }) },
        ...(create.length > 0 ? [{ label: m.newMenu, icon: 'plus' as const, items: create }] : []),
        { label: m.deleteConnection, icon: 'trash', onSelect: () => setDeleting(connection) },
      ]
    }
    const connection = connections.find((c) => c.id === node.connectionId)
    if (node.kind !== 'database' || !connection || !DUMP_ENGINES.includes(connection.engine)) return []
    const open = (mode: 'export' | 'import') => () =>
      setDump({ mode, connection, database: node.name, refresh })
    return [
      { label: m.exportDatabase, onSelect: open('export') },
      { label: m.importSqlFile, onSelect: open('import') },
    ]
  }

  const removeConnection = async (id: string) => {
    await remove(id)
    setTabs((state) => closeConnectionTabs(state, id))
  }

  return (
    <div className={styles.app}>
      <TitleBar
        connection={activeConnection}
        path={activePath}
        onNewQuery={newQuery}
        onOpenSettings={() => setSettingsOpen(true)}
        onSearch={() => setPaletteOpen(true)}
        explorerOpen={explorerOpen}
        onToggleExplorer={() => setExplorerOpen((open) => !open)}
        onEditConnection={(connection) => setDialog({ connection })}
      />

      <div className={styles.body}>
        <div className={styles.explorerSlot} hidden={!explorerOpen} style={{ width: explorerWidth }}>
          <ResizeHandle
            edge="right"
            label={m.resizeExplorer}
            onResize={setExplorerWidth}
            onReset={resetExplorerWidth}
          />
          <Explorer
            connections={connections}
            onSelect={({ connectionId, path }) => setSelection({ connectionId, path })}
            onOpen={(node) => {
              if (node.kind === 'table' || node.kind === 'view') {
                setTabs((state) => openTableTab(state, node.connectionId, tableRefFromPath(node.path)))
              }
              const connection = connections.find((c) => c.id === node.connectionId)
              if (node.kind === 'database' && connection?.engine === 'redis') {
                setTabs((state) => openKeyspaceTab(state, connection.id, connection.name, node.name))
              }
            }}
            diagramOf={(node) => {
              const connection = connections.find((c) => c.id === node.connectionId)
              if (!connection || !hasDiagram(connection.engine)) return null
              const [database, schemaName] = node.path
              // Schemaless engines (MySQL) draw a database; the others draw a schema.
              const schema =
                node.kind === 'schema'
                  ? schemaName
                  : node.kind === 'database' && defaultSchema(connection.engine, database) === database
                    ? database
                    : null
              if (!schema) return null
              return () => setTabs((state) => openDiagramTab(state, m, connection.id, database, schema))
            }}
            onNewConnection={() => setDialog({})}
            menuOf={menuOf}
          />
        </div>
        <main className={`island ${styles.main}`}>
          {loadError ? (
            <p className={styles.error}>{m.loadConnectionsFailed(loadError)}</p>
          ) : (
            <Workspace
              tabs={tabs.tabs}
              activeId={tabs.activeId}
              onSelect={(id) => setTabs((state) => ({ ...state, activeId: id }))}
              onClose={(id) => setTabs((state) => closeTab(state, id))}
              onNewQuery={newQuery}
              connectionOf={(id) => connections.find((c) => c.id === id)}
              onOpenTable={(connectionId, table, filter) =>
                setTabs((state) => openTableTab(state, connectionId, table, filter))
              }
              onOpenDiagram={(connectionId, database, schema) =>
                setTabs((state) => openDiagramTab(state, m, connectionId, database, schema))
              }
              onOpenQuery={(entry) =>
                setTabs((state) =>
                  newQueryTab(
                    state,
                    m,
                    entry.connectionId,
                    entry.database ?? undefined,
                    entry.sql,
                    entry.container ?? undefined,
                  ),
                )
              }
            />
          )}
        </main>
      </div>

      {dialog && (
        <ConnectionDialog connection={dialog.connection} onSave={save} onClose={() => setDialog(null)} />
      )}

      {creating && (
        <NewObjectDialog
          connection={creating.connection}
          kind={creating.kind}
          onClose={() => setCreating(null)}
          onCreated={creating.refresh}
        />
      )}
      {deleting && (
        <DeleteConnectionDialog
          connection={deleting}
          onDelete={removeConnection}
          onClose={() => setDeleting(null)}
        />
      )}

      {dump?.mode === 'export' && (
        <ExportDatabaseDialog
          connectionId={dump.connection.id}
          connectionName={dump.connection.name}
          database={dump.database}
          onClose={() => setDump(null)}
        />
      )}
      {dump?.mode === 'import' && (
        <ImportDatabaseDialog
          connectionId={dump.connection.id}
          connectionName={dump.connection.name}
          database={dump.database}
          production={dump.connection.environment === 'production'}
          onClose={() => setDump(null)}
          onImported={dump.refresh}
        />
      )}

      {paletteOpen && (
        <CommandPalette
          source={
            activeConnection && {
              connectionId: activeConnection.id,
              connectionName: activeConnection.name,
              database: activePath[0],
            }
          }
          commands={commands}
          onOpenTable={(connectionId, table) => setTabs((state) => openTableTab(state, connectionId, table))}
          onClose={() => setPaletteOpen(false)}
        />
      )}

      {settingsOpen && (
        <SettingsDialog
          theme={theme.preference}
          onThemeChange={theme.choose}
          onClose={() => setSettingsOpen(false)}
        />
      )}
    </div>
  )
}
