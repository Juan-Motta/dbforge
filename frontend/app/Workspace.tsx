import { memo, useState } from 'react'
import { IconButton } from '../components/Button'
import { hasDiagram, sqlDialectOf, type Connection } from '../domain/connection'
import { Icon } from '../components/Icon'
import { TabStrip } from '../components/TabStrip'
import { DiagramView } from '../features/diagram/DiagramView'
import { ExportDialog, type ExportSource } from '../features/export/ExportDialog'
import type { HistoryEntry } from '../features/history/api'
import type { RowFilter } from '../features/table-data/rowFilter'
import type { TableRef } from '../domain/table'
import { KeyBrowser } from '../features/keyspace/KeyBrowser'
import { QueryConsole } from '../features/query/QueryConsole'
import { useMessages } from '../lib/i18n/useLanguage'
import { messages } from './messages'
import { TablePage } from './TablePage'
import type { Tab, TableTabView } from './tabs'
import { confirmingTag } from '../features/tags/tags'
import { useTags } from '../features/tags/useTags'
import styles from './App.module.css'

/** What a tab's content asks of the workspace: other tabs to open, its own view to keep. */
export interface TabActions {
  /** Opens a query from the history in a new console. */
  openQuery: (entry: HistoryEntry) => void
  openDiagram: (connectionId: string, database: string, schema: string) => void
  openTable: (connectionId: string, table: TableRef, filter: RowFilter) => void
  setTableView: (tabId: string, view: TableTabView) => void
}

interface WorkspaceProps {
  tabs: Tab[]
  activeId: string | null
  onSelect: (id: string) => void
  onClose: (id: string) => void
  /** Undefined when there is no connection to query. */
  onNewQuery?: () => void
  connections: Connection[]
  /** The same object between renders (see `TabContent`). */
  actions: TabActions
}

const TAB_ICONS = { query: 'code', table: 'table', diagram: 'diagram', keyspace: 'database' } as const

/** The main island: open query consoles, tables and diagrams, one visible at a time. */
export function Workspace({
  tabs,
  activeId,
  onSelect,
  onClose,
  onNewQuery,
  connections,
  actions,
}: WorkspaceProps) {
  const [exporting, setExporting] = useState<ExportSource | null>(null)
  const m = useMessages(messages)

  if (tabs.length === 0) {
    return (
      <p className={styles.empty}>
        {m.emptyBefore}
        <strong>{m.newQuery}</strong>
        {m.emptyAfter}
      </p>
    )
  }

  return (
    <div className={styles.workspace}>
      <TabStrip
        tabs={tabs.map((tab) => ({ ...tab, icon: TAB_ICONS[tab.kind] }))}
        activeId={activeId}
        onSelect={onSelect}
        onClose={onClose}
      >
        <IconButton label={m.newQuery} disabled={!onNewQuery} onClick={onNewQuery} className={styles.newTab}>
          <Icon name="plus" />
        </IconButton>
      </TabStrip>

      {/* Inactive tabs stay mounted so their editor text and results survive switching. */}
      {tabs.map((tab) => (
        <div key={tab.id} className={styles.tabContent} hidden={tab.id !== activeId}>
          <TabContent tab={tab} connections={connections} actions={actions} onExport={setExporting} />
        </div>
      ))}

      {exporting && <ExportDialog source={exporting} onClose={() => setExporting(null)} />}
    </div>
  )
}

interface TabContentProps {
  tab: Tab
  connections: Connection[]
  actions: TabActions
  onExport: (source: ExportSource) => void
}

/**
 * What one tab shows. Every tab stays mounted, so this is drawn again only
 * when its own tab, the connections or the tags change: what happens elsewhere
 * in the app (another tab, the explorer, a notice) leaves an open table as it is.
 */
const TabContent = memo(function TabContent({ tab, connections, actions, onExport }: TabContentProps) {
  const { tags, loaded } = useTags()
  const connection = connections.find((c) => c.id === tab.connectionId)
  switch (tab.kind) {
    case 'query':
      return (
        <QueryConsole
          sessionId={tab.id}
          connectionId={tab.connectionId}
          database={tab.database}
          container={tab.container}
          initialSql={tab.initialSql}
          onOpenQuery={actions.openQuery}
          canOpen={(entry) => connections.some((c) => c.id === entry.connectionId)}
          dialect={sqlDialectOf(connection?.engine)}
          onExportResult={(result) =>
            onExport({ kind: 'rows', name: 'query_result', columns: result.columns, rows: result.rows })
          }
        />
      )
    case 'diagram':
      return <DiagramView connectionId={tab.connectionId} database={tab.database} schema={tab.schema} />
    case 'keyspace':
      return (
        <KeyBrowser
          tabId={tab.id}
          connectionId={tab.connectionId}
          database={tab.database}
          address={addressOf(connection)}
        />
      )
    case 'table':
      return (
        <TablePage
          connectionId={tab.connectionId}
          table={tab.table}
          confirmTag={confirmingTag(tags, connection?.environment ?? null, loaded)}
          dialect={sqlDialectOf(connection?.engine)}
          diagram={hasDiagram(connection?.engine ?? 'redis')}
          onOpenSchemaDiagram={() =>
            actions.openDiagram(tab.connectionId, tab.table.database, tab.table.schema)
          }
          initialFilter={tab.filter}
          view={tab.view ?? 'data'}
          onViewChange={(view) => actions.setTableView(tab.id, view)}
          onOpenTable={(table, filter) => actions.openTable(tab.connectionId, table, filter)}
          onExport={(filter) =>
            onExport({ kind: 'table', connectionId: tab.connectionId, table: tab.table, filter })
          }
        />
      )
  }
})

/** `host:port`, or the URL's host for a connection given as a URL. */
function addressOf(connection: Connection | undefined): string {
  if (!connection) return ''
  if (connection.host.includes('://'))
    return connection.host.replace(/^[a-z]+:\/\/([^@]*@)?/, '').replace(/\/.*$/, '')
  return `${connection.host}:${connection.port}`
}
