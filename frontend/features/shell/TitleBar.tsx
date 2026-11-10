import { Button, IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import type { Connection } from '../../domain/connection'
import { useMessages } from '../../lib/i18n/useLanguage'
import { ConnectionDetails } from './ConnectionDetails'
import { messages } from './messages'
import { hasOverlayTitleBar } from './platform'
import styles from './Shell.module.css'

interface TitleBarProps {
  connection?: Connection
  /** Database and schema of the current selection. */
  path: string[]
  /** Undefined when there is no connection to query. */
  onNewQuery?: () => void
  onOpenSettings: () => void
  onSearch: () => void
  explorerOpen: boolean
  onToggleExplorer: () => void
  onEditConnection: (connection: Connection) => void
}

export function TitleBar({
  connection,
  path,
  onNewQuery,
  onOpenSettings,
  onSearch,
  explorerOpen,
  onToggleExplorer,
  onEditConnection,
}: TitleBarProps) {
  const m = useMessages(messages)
  return (
    <header
      className={styles.titleBar}
      data-tauri-drag-region
      style={{ paddingLeft: hasOverlayTitleBar ? 84 : 14 }}
    >
      <span className={styles.brand} data-tauri-drag-region>
        <span className={styles.brandMark}>
          <Icon name="database" size={14} />
        </span>
        DBForge
      </span>
      <IconButton
        label={explorerOpen ? m.hideExplorer : m.showExplorer}
        title={explorerOpen ? m.hideExplorerTitle : m.showExplorerTitle}
        active={explorerOpen}
        onClick={onToggleExplorer}
      >
        <Icon name="sidebar" />
      </IconButton>

      {connection ? (
        <ConnectionDetails connection={connection} onEdit={() => onEditConnection(connection)} />
      ) : (
        <span className="muted">{m.noConnection}</span>
      )}

      {path.length > 0 && (
        <Button variant="ghost">
          {path.slice(0, 2).map((segment, i) => (
            <span key={segment} className={styles.crumb}>
              {i > 0 && <span className="muted">/</span>}
              <Icon name={i === 0 ? 'database' : 'schema'} />
              {segment}
            </span>
          ))}
          <Icon name="chevronDown" size={12} className="muted" />
        </Button>
      )}

      <span className={styles.spacer} data-tauri-drag-region />

      <Button className={styles.search} onClick={onSearch}>
        <Icon name="search" />
        {m.searchEverywhere}
        <kbd className={styles.kbd}>⌘K</kbd>
      </Button>
      <Button
        variant="primary"
        disabled={!onNewQuery}
        title={onNewQuery ? undefined : m.selectConnectionFirst}
        onClick={onNewQuery}
      >
        <Icon name="plus" />
        {m.newQuery}
      </Button>
      <IconButton label={m.settings} onClick={onOpenSettings}>
        <Icon name="settings" />
      </IconButton>
    </header>
  )
}
