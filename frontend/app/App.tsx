import { useEffect, useState } from 'react'
import { defaultDatabase, defaultSchema, hasDiagram, type Connection } from '../domain/connection'
import { tableRefFromPath } from '../domain/table'
import { ConnectionDialog } from '../features/connections/ConnectionDialog'
import { useConnections } from '../features/connections/useConnections'
import { Explorer } from '../features/explorer/Explorer'
import { HistoryPanel } from '../features/history/HistoryPanel'
import { CommandPalette, type Command } from '../features/palette/CommandPalette'
import { SettingsDialog } from '../features/settings/SettingsDialog'
import { useTheme } from '../features/settings/useTheme'
import { ActivityRail, type Activity } from '../features/shell/ActivityRail'
import { StatusBar } from '../features/shell/StatusBar'
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
  const [activity, setActivity] = useState<Activity>('explorer')
  const [selection, setSelection] = useState<Selection | null>(null)
  const [dialog, setDialog] = useState<DialogState>(null)
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

  // The rail's diagram and search buttons act instead of switching the side panel.
  const onActivity = (next: Activity) => {
    if (next === 'diagram') openDiagram?.()
    else if (next === 'search') setPaletteOpen(true)
    else setActivity(next)
  }

  // ⌘K / Ctrl+K toggles the palette from anywhere, the SQL editor included.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setPaletteOpen((open) => !open)
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
      />

      <div className={styles.body}>
        <ActivityRail active={activity} onChange={onActivity} />
        {activity === 'explorer' && (
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
            onNewConnection={() => setDialog({})}
            onEditConnection={(connection) => setDialog({ connection })}
          />
        )}
        {activity === 'history' && (
          <HistoryPanel
            canOpen={(entry) => connections.some((c) => c.id === entry.connectionId)}
            onOpen={(entry) =>
              setTabs((state) =>
                newQueryTab(state, entry.connectionId, entry.database ?? undefined, entry.sql),
              )
            }
          />
        )}
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
            />
          )}
        </main>
      </div>

      <StatusBar connection={activeConnection} path={activePath} />

      {dialog && (
        <ConnectionDialog
          connection={dialog.connection}
          onSave={save}
          onDelete={removeConnection}
          onClose={() => setDialog(null)}
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
