import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  ENGINES,
  defaultDatabase,
  defaultSchema,
  hasDiagram,
  type Connection,
  type NewObjectKind,
} from '../domain/connection'
import { tableRefFromPath } from '../domain/table'
import { NewObjectDialog } from '../features/admin/NewObjectDialog'
import { messages as adminMessages } from '../features/admin/messages'
import { ConnectionDialog } from '../features/connections/ConnectionDialog'
import { DeleteConnectionDialog } from '../features/connections/DeleteConnectionDialog'
import { ExportDatabaseDialog, ImportDatabaseDialog } from '../features/dump/DumpDialogs'
import { useConnections } from '../features/connections/useConnections'
import type { MenuItem } from '../components/ContextMenu'
import { ResizeHandle } from '../components/ResizeHandle'
import { usePanelSize } from '../components/usePanelSize'
import { Explorer } from '../features/explorer/Explorer'
import type { TreeNode } from '../features/explorer/tree'
import { CommandPalette, type Command } from '../features/palette/CommandPalette'
import { SettingsDialog } from '../features/settings/SettingsDialog'
import { StatementConsole } from '../features/statement-log/StatementConsole'
import { ApprovalDialog } from '../features/mcp/ApprovalDialog'
import { useApprovals } from '../features/mcp/useApprovals'
import { useStoredFlag } from '../components/useStoredFlag'
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
  setTableView,
} from './tabs'
import { useMessages } from '../lib/i18n/useLanguage'
import { messages } from './messages'
import { explorerMenu, type MenuActions } from './explorerMenu'
import { Workspace, type TabActions } from './Workspace'
import { confirmingTag } from '../features/tags/tags'
import { useTagList } from '../features/tags/useTagList'
import { TagsContext } from '../features/tags/useTags'
import { Notice, type NoticeState } from '../components/Notice'
import { ExportDialog, type ExportSource } from '../features/export/ExportDialog'
import { NO_FILTER } from '../features/table-data/rowFilter'
import { structureApi } from '../features/table-structure/api'
import { copyText } from '../lib/clipboard'
import { errorMessage } from '../lib/errors'
import styles from './App.module.css'

/** `null` while closed; `{}` for a new connection. */
type DialogState = { connection?: Connection } | null

interface Selection {
  connectionId: string
  path: string[]
}

export function App() {
  const { connections, loadError, save, remove, duplicate, reload } = useConnections()
  // Deleting a tag also clears it from connections, so they are read again.
  const tagsState = useTagList(reload)
  // Hidden with ⌘B for more room; it stays mounted so its expanded nodes survive.
  const [explorerOpen, setExplorerOpen] = useState(true)
  const [explorerWidth, setExplorerWidth, resetExplorerWidth] = usePanelSize(
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
  // Changes agents ask to make on connections whose tag asks first.
  const approvals = useApprovals()
  const [statementsOpen, setStatementsOpen] = useStoredFlag('dbforge.statementsOpen', false)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const theme = useTheme()
  const m = useMessages(messages)
  const adminM = useMessages(adminMessages)
  // A table being exported from the explorer's menu.
  const [exporting, setExporting] = useState<ExportSource | null>(null)
  const [notice, setNotice] = useState<NoticeState | null>(null)

  // A notice goes away by itself; a newer one restarts the wait.
  useEffect(() => {
    if (!notice) return
    const timer = setTimeout(() => setNotice((current) => (current?.id === notice.id ? null : current)), 2500)
    return () => clearTimeout(timer)
  }, [notice])

  const notify = useCallback(
    (text: string, tone: NoticeState['tone'] = 'info') =>
      setNotice((current) => ({ id: (current?.id ?? 0) + 1, text, tone })),
    [],
  )
  const copyStatement = useCallback(
    (text: string) =>
      void copyText(text).then(
        () => notify(m.copied),
        (error) => notify(errorMessage(error), 'error'),
      ),
    [notify, m],
  )

  const activeConnection = connections.find((c) => c.id === selection?.connectionId)
  const activePath = activeConnection ? (selection?.path ?? []) : []

  // A new query runs against the selected connection and database. Redis has
  // no query consoles: its key browser holds the CLI.
  const newQuery = activeConnection
    ? ENGINES[activeConnection.engine].keyBrowser
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
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'j') {
        event.preventDefault()
        setStatementsOpen((open) => !open)
      }
    }
    window.addEventListener('keydown', onKeyDown, { capture: true })
    return () => window.removeEventListener('keydown', onKeyDown, { capture: true })
  }, [setStatementsOpen])

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
    const connection = connections.find((c) => c.id === node.connectionId)
    if (!connection) return []
    const actions: MenuActions = {
      editConnection: (c) => setDialog({ connection: c }),
      duplicateConnection: (c) =>
        void duplicate(c.id, m.duplicateName(c.name)).catch((error) => notify(errorMessage(error), 'error')),
      deleteConnection: setDeleting,
      newObject: (c, kind) => setCreating({ connection: c, kind, refresh }),
      newQuery: (c, database, sql, container) =>
        setTabs((state) => newQueryTab(state, m, c.id, database, sql, container)),
      openTable: (c, table, view) => setTabs((state) => openTableTab(state, c.id, table, undefined, view)),
      openDiagram: (c, database, schema) =>
        setTabs((state) => openDiagramTab(state, m, c.id, database, schema)),
      exportTable: (c, table) =>
        setExporting({ kind: 'table', connectionId: c.id, table, filter: NO_FILTER }),
      dump: (mode, c, database) => setDump({ mode, connection: c, database, refresh }),
      copy: (text) =>
        void copyText(text).then(
          () => notify(m.copied),
          (error) => notify(errorMessage(error), 'error'),
        ),
      structure: (c, table) => structureApi.get(c.id, table),
      refresh,
    }
    return explorerMenu(node, connection, actions, m, adminM)
  }

  // What a tab's content asks of the workspace. The same object between renders,
  // so a tab is drawn again only when it changes itself.
  const tabActions = useMemo<TabActions>(
    () => ({
      openTable: (connectionId, table, filter) =>
        setTabs((state) => openTableTab(state, connectionId, table, filter)),
      openDiagram: (connectionId, database, schema) =>
        setTabs((state) => openDiagramTab(state, m, connectionId, database, schema)),
      setTableView: (id, view) => setTabs((state) => setTableView(state, id, view)),
      openQuery: (entry) =>
        setTabs((state) =>
          newQueryTab(
            state,
            m,
            entry.connectionId,
            entry.database ?? undefined,
            entry.sql,
            entry.container ?? undefined,
          ),
        ),
    }),
    [m],
  )

  const removeConnection = async (id: string) => {
    await remove(id)
    setTabs((state) => closeConnectionTabs(state, id))
  }

  return (
    <TagsContext value={tagsState}>
      <div className={styles.app}>
        <TitleBar
          connection={activeConnection}
          path={activePath}
          onNewQuery={newQuery}
          onOpenSettings={() => setSettingsOpen(true)}
          statementsOpen={statementsOpen}
          onToggleStatements={() => setStatementsOpen((open) => !open)}
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
                if (node.kind === 'database' && connection && ENGINES[connection.engine].keyBrowser) {
                  setTabs((state) => openKeyspaceTab(state, connection.id, connection.name, node.name))
                }
              }}
              onNewConnection={() => setDialog({})}
              menuOf={menuOf}
            />
          </div>
          <div className={styles.column}>
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
                  connections={connections}
                  actions={tabActions}
                />
              )}
            </main>
            {statementsOpen && (
              <StatementConsole
                connectionOf={(id) => connections.find((c) => c.id === id)}
                onCopy={copyStatement}
                onClose={() => setStatementsOpen(false)}
              />
            )}
          </div>
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

        {exporting && <ExportDialog source={exporting} onClose={() => setExporting(null)} />}
        {notice && <Notice notice={notice} />}

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
            confirmTag={confirmingTag(tagsState.tags, dump.connection.environment, tagsState.loaded)}
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
            onOpenTable={(connectionId, table) =>
              setTabs((state) => openTableTab(state, connectionId, table))
            }
            onClose={() => setPaletteOpen(false)}
          />
        )}

        {approvals.current && (
          <ApprovalDialog
            key={approvals.current.id}
            request={approvals.current}
            engine={connections.find((c) => c.id === approvals.current?.connectionId)?.engine}
            onAnswer={(approved) => approvals.current && approvals.answer(approvals.current.id, approved)}
          />
        )}

        {settingsOpen && (
          <SettingsDialog
            theme={theme.preference}
            onThemeChange={theme.choose}
            onConnectionsChanged={() => void reload()}
            onClose={() => setSettingsOpen(false)}
          />
        )}
      </div>
    </TagsContext>
  )
}
