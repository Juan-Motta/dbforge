import { useEffect, useState } from 'react'
import type { Connection } from '../domain/connection'
import { ConnectionDialog } from '../features/connections/ConnectionDialog'
import { useConnections } from '../features/connections/useConnections'
import { Explorer } from '../features/explorer/Explorer'
import { CommandPalette, type Command } from '../features/palette/CommandPalette'
import { SettingsDialog } from '../features/settings/SettingsDialog'
import { useTheme } from '../features/settings/useTheme'
import { ActivityRail, type Activity } from '../features/shell/ActivityRail'
import { StatusBar } from '../features/shell/StatusBar'
import { TitleBar } from '../features/shell/TitleBar'
import { closeConnectionTabs, closeTab, newQueryTab, NO_TABS, openTableTab } from './tabs'
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

  // A new query runs against the selected connection and database.
  const newQuery = activeConnection
    ? () => setTabs((state) => newQueryTab(state, activeConnection.id, activePath[0]))
    : undefined

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
        <ActivityRail active={activity} onChange={setActivity} />
        {activity === 'explorer' && (
          <Explorer
            connections={connections}
            onSelect={({ connectionId, path }) => setSelection({ connectionId, path })}
            onOpen={(node) => {
              const [database, schema, name] = node.path
              if (node.kind === 'table' || node.kind === 'view') {
                setTabs((state) => openTableTab(state, node.connectionId, { database, schema, name }))
              }
            }}
            onNewConnection={() => setDialog({})}
            onEditConnection={(connection) => setDialog({ connection })}
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
