import { IconButton } from '../components/Button'
import { Icon } from '../components/Icon'
import { TabStrip } from '../components/TabStrip'
import { QueryConsole } from '../features/query/QueryConsole'
import { TableView } from '../features/table-data/TableView'
import type { Tab } from './tabs'
import styles from './App.module.css'

interface WorkspaceProps {
  tabs: Tab[]
  activeId: string | null
  onSelect: (id: string) => void
  onClose: (id: string) => void
  /** Undefined when there is no connection to query. */
  onNewQuery?: () => void
}

/** The main island: open query consoles and tables, one visible at a time. */
export function Workspace({ tabs, activeId, onSelect, onClose, onNewQuery }: WorkspaceProps) {
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
        tabs={tabs.map((tab) => ({ ...tab, icon: tab.kind === 'query' ? 'code' : 'table' }))}
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
            <QueryConsole sessionId={tab.id} connectionId={tab.connectionId} database={tab.database} />
          ) : (
            <TableView connectionId={tab.connectionId} table={tab.table} />
          )}
        </div>
      ))}
    </div>
  )
}
