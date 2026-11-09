import { useEffect, useState } from 'react'
import {
  defaultDatabase,
  defaultSchema,
  DUMP_ENGINES,
  hasDiagram,
  type Connection,
} from '../domain/connection'
import { tableRefFromPath } from '../domain/table'
import { ConnectionDialog } from '../features/connections/ConnectionDialog'
import { ExportDatabaseDialog, ImportDatabaseDialog } from '../features/dump/DumpDialogs'
import { useConnections } from '../features/connections/useConnections'
import { ResizeHandle } from '../components/ResizeHandle'
import { usePanelWidth } from '../components/usePanelWidth'
import { Explorer } from '../features/explorer/Explorer'
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
  const [tabs, setTabs] = useState(NO_TABS)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const theme = useTheme()

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
                activeConnection.id,
                activePath[0] ?? (activeConnection.database || undefined),
                undefined,
                activePath[1],
              ),
            )
        : () => setTabs((state) => newQueryTab(state, activeConnection.id, activePath[0]))
    : undefined

  // The diagram of the selected schema, or the default one when only a connection or database is selected.
  const diagramDatabase = activePath[0] ?? (activeConnection && defaultDatabase(activeConnection))
  const diagramSchema =
    activeConnection && diagramDatabase
      ? (activePath[1] ?? defaultSchema(activeConnection.engine, diagramDatabase))
      : undefined
  const openDiagram =
    activeConnection && diagramDatabase && hasDiagram(activeConnection.engine)
      ? () => setTabs((state) => openDiagramTab(state, activeConnection.id, diagramDatabase, diagramSchema!))
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
      ? [{ id: 'new-query', label: 'New query', detail: activeConnection?.name, run: newQuery }]
      : []),
    ...(openDiagram
      ? [
          {
            id: 'diagram',
            label: 'Show ER diagram',
            detail:
              diagramDatabase === diagramSchema ? diagramDatabase : `${diagramDatabase}.${diagramSchema}`,
            run: openDiagram,
          },
        ]
      : []),
    { id: 'new-connection', label: 'New connection', run: () => setDialog({}) },
    { id: 'theme-dark', label: 'Theme: Dark', run: () => theme.choose('dark') },
    { id: 'theme-light', label: 'Theme: Light', run: () => theme.choose('light') },
    { id: 'theme-system', label: 'Theme: Match system', run: () => theme.choose('system') },
    { id: 'settings', label: 'Open settings', run: () => setSettingsOpen(true) },
  ]

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
            label="Resize the explorer"
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
              return () => setTabs((state) => openDiagramTab(state, connection.id, database, schema))
            }}
            onNewConnection={() => setDialog({})}
            onEditConnection={(connection) => setDialog({ connection })}
            menuOf={(node, refresh) => {
              const connection = connections.find((c) => c.id === node.connectionId)
              if (node.kind !== 'database' || !connection || !DUMP_ENGINES.includes(connection.engine))
                return []
              const open = (mode: 'export' | 'import') => () =>
                setDump({ mode, connection, database: node.name, refresh })
              return [
                { label: 'Export database…', onSelect: open('export') },
                { label: 'Import SQL file…', onSelect: open('import') },
              ]
            }}
          />
        </div>
        <main className={`island ${styles.main}`}>
          {loadError ? (
            <p className={styles.error}>Could not load connections: {loadError}</p>
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
                setTabs((state) => openDiagramTab(state, connectionId, database, schema))
              }
              onOpenQuery={(entry) =>
                setTabs((state) =>
                  newQueryTab(
                    state,
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
        <ConnectionDialog
          connection={dialog.connection}
          onSave={save}
          onDelete={removeConnection}
          onClose={() => setDialog(null)}
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
