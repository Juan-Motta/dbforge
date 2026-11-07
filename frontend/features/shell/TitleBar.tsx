import { Badge } from '../../components/Badge'
import { Button, IconButton } from '../../components/Button'
import { EngineLogo } from '../../components/EngineLogo'
import { Icon } from '../../components/Icon'
import type { Connection } from '../../domain/connection'
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
}

export function TitleBar({
  connection,
  path,
  onNewQuery,
  onOpenSettings,
  onSearch,
  explorerOpen,
  onToggleExplorer,
}: TitleBarProps) {
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
        label={explorerOpen ? 'Hide explorer (⌘B)' : 'Show explorer (⌘B)'}
        title={explorerOpen ? 'Hide explorer ⌘B' : 'Show explorer ⌘B'}
        active={explorerOpen}
        onClick={onToggleExplorer}
      >
        <Icon name="sidebar" />
      </IconButton>

      {connection ? (
        <Button className={styles.connection}>
          <EngineLogo engine={connection.engine} />
          {connection.name}
          {connection.environment === 'production' && <Badge tone="err">PROD</Badge>}
          {connection.environment === 'staging' && <Badge tone="warn">STAGE</Badge>}
          <Icon name="chevronDown" size={12} className="muted" />
        </Button>
      ) : (
        <span className="muted">No connection selected</span>
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
        Search everywhere…
        <kbd className={styles.kbd}>⌘K</kbd>
      </Button>
      <Button
        variant="primary"
        disabled={!onNewQuery}
        title={onNewQuery ? undefined : 'Select a connection first'}
        onClick={onNewQuery}
      >
        <Icon name="plus" />
        New query
      </Button>
      <IconButton label="Settings" onClick={onOpenSettings}>
        <Icon name="settings" />
      </IconButton>
    </header>
  )
}
