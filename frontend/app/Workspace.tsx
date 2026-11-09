import { useState } from 'react'
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
import { TablePage } from './TablePage'
import type { Tab } from './tabs'
import styles from './App.module.css'

interface WorkspaceProps {
  tabs: Tab[]
  activeId: string | null
  onSelect: (id: string) => void
  onClose: (id: string) => void
  /** Undefined when there is no connection to query. */
  onNewQuery?: () => void
  connectionOf: (connectionId: string) => Connection | undefined
  /** Opens a query from the history in a new console. */
  onOpenQuery: (entry: HistoryEntry) => void
  onOpenDiagram: (connectionId: string, database: string, schema: string) => void
  onOpenTable: (connectionId: string, table: TableRef, filter: RowFilter) => void
}

const TAB_ICONS = { query: 'code', table: 'table', diagram: 'diagram', keyspace: 'database' } as const

/** The main island: open query consoles, tables and diagrams, one visible at a time. */
export function Workspace({
  tabs,
  activeId,
  onSelect,
  onClose,
  onNewQuery,
  connectionOf,
  onOpenQuery,
  onOpenDiagram,
  onOpenTable,
}: WorkspaceProps) {
  const [exporting, setExporting] = useState<ExportSource | null>(null)

  if (tabs.length === 0) {
    return (
      <p className={styles.empty}>
        Double-click a table to see its rows, or press <strong>New query</strong>.
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
        <IconButton label="New query" disabled={!onNewQuery} onClick={onNewQuery} className={styles.newTab}>
          <Icon name="plus" />
        </IconButton>
      </TabStrip>

      {/* Inactive tabs stay mounted so their editor text and results survive switching. */}
      {tabs.map((tab) => (
        <div key={tab.id} className={styles.tabContent} hidden={tab.id !== activeId}>
          {tab.kind === 'query' ? (
            <QueryConsole
              sessionId={tab.id}
              connectionId={tab.connectionId}
              database={tab.database}
              container={tab.container}
              initialSql={tab.initialSql}
              onOpenQuery={onOpenQuery}
              canOpen={(entry) => connectionOf(entry.connectionId) !== undefined}
              dialect={sqlDialectOf(connectionOf(tab.connectionId)?.engine)}
              onExportResult={(result) =>
                setExporting({
                  kind: 'rows',
                  name: 'query_result',
                  columns: result.columns,
                  rows: result.rows,
                })
              }
            />
          ) : tab.kind === 'diagram' ? (
            <DiagramView connectionId={tab.connectionId} database={tab.database} schema={tab.schema} />
          ) : tab.kind === 'keyspace' ? (
            <KeyBrowser
              tabId={tab.id}
              connectionId={tab.connectionId}
              database={tab.database}
              address={addressOf(connectionOf(tab.connectionId))}
            />
          ) : (
            <TablePage
              connectionId={tab.connectionId}
              table={tab.table}
              production={connectionOf(tab.connectionId)?.environment === 'production'}
              dialect={sqlDialectOf(connectionOf(tab.connectionId)?.engine)}
              diagram={hasDiagram(connectionOf(tab.connectionId)?.engine ?? 'redis')}
              onOpenSchemaDiagram={() =>
                onOpenDiagram(tab.connectionId, tab.table.database, tab.table.schema)
              }
              initialFilter={tab.filter}
              onOpenTable={(table, filter) => onOpenTable(tab.connectionId, table, filter)}
              onExport={(filter) =>
                setExporting({ kind: 'table', connectionId: tab.connectionId, table: tab.table, filter })
              }
            />
          )}
        </div>
      ))}

      {exporting && <ExportDialog source={exporting} onClose={() => setExporting(null)} />}
    </div>
  )
}

/** `host:port`, or the URL's host for a connection given as a URL. */
function addressOf(connection: Connection | undefined): string {
  if (!connection) return ''
  if (connection.host.includes('://'))
    return connection.host.replace(/^[a-z]+:\/\/([^@]*@)?/, '').replace(/\/.*$/, '')
  return `${connection.host}:${connection.port}`
}
