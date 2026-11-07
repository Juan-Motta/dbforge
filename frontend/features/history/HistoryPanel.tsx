import { useState, type MouseEvent } from 'react'
import { IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import type { HistoryEntry } from './api'
import {
  firstLine,
  formatDuration,
  formatTime,
  groupByDay,
  inScope,
  outcomeLabel,
  type HistoryScope,
} from './format'
import { useHistory } from './useHistory'
import styles from './HistoryPanel.module.css'

interface HistoryPanelProps {
  /** The console's connection: its runs are shown first, or alone. */
  connectionId: string
  /** Changes after each run, so the newest one appears. */
  reloadKey: unknown
  /** Puts the entry's SQL in the console, without running it. */
  onLoad: (entry: HistoryEntry) => void
  /** Opens the entry's SQL in a new console on its own connection. */
  onOpenNew: (entry: HistoryEntry) => void
  /** False once the entry's connection has been deleted. */
  canOpen: (entry: HistoryEntry) => boolean
  onClose: () => void
}

/** The queries run from consoles, newest first, grouped by day, beside a console. */
export function HistoryPanel({
  connectionId,
  reloadKey,
  onLoad,
  onOpenNew,
  canOpen,
  onClose,
}: HistoryPanelProps) {
  const [search, setSearch] = useState('')
  const [errorsOnly, setErrorsOnly] = useState(false)
  const [scope, setScope] = useState<HistoryScope>('connection')
  const { entries: all, loadedAt, error, loading } = useHistory(search, errorsOnly, reloadKey)
  const entries = all && inScope(all, scope, connectionId)

  return (
    <aside className={styles.panel} aria-label="Query history">
      <div className={styles.header}>
        <span className={styles.title}>History</span>
        <IconButton label="Close history" onClick={onClose}>
          <Icon name="close" />
        </IconButton>
      </div>

      <div className={styles.filters}>
        <label className={styles.search}>
          <Icon name="search" className={styles.searchIcon} />
          <input
            type="search"
            aria-label="Search SQL text"
            placeholder="Search SQL text…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </label>
        <button
          type="button"
          aria-pressed={errorsOnly}
          className={styles.toggle}
          onClick={() => setErrorsOnly((current) => !current)}
        >
          Errors
        </button>
      </div>
      <div className={styles.scope} role="group" aria-label="Whose runs">
        <button type="button" aria-pressed={scope === 'connection'} onClick={() => setScope('connection')}>
          This connection
        </button>
        <button type="button" aria-pressed={scope === 'all'} onClick={() => setScope('all')}>
          All
        </button>
      </div>

      <div className={styles.list} aria-busy={loading}>
        {error ? (
          <p className={styles.error}>Could not load the history: {error}</p>
        ) : entries?.length === 0 ? (
          <p className={styles.empty}>
            {search || errorsOnly ? 'No runs match.' : 'Queries you run appear here.'}
          </p>
        ) : (
          entries &&
          groupByDay(entries, new Date(loadedAt)).map((group) => (
            <section key={group.label} aria-label={group.label}>
              <h3 className={styles.day}>{group.label}</h3>
              {group.entries.map((entry) => (
                <Entry
                  key={entry.id}
                  entry={entry}
                  onClick={(event) => {
                    // ⌘-click (Ctrl-click) opens a new console, as links open a new tab.
                    if ((event.metaKey || event.ctrlKey) && canOpen(entry)) onOpenNew(entry)
                    else onLoad(entry)
                  }}
                />
              ))}
            </section>
          ))
        )}
      </div>
      <p className={styles.hint}>Click to load into the editor · ⌘-click opens a new tab</p>
    </aside>
  )
}

function Entry({ entry, onClick }: { entry: HistoryEntry; onClick: (event: MouseEvent) => void }) {
  const failed = entry.error !== null
  const tooltip = [entry.sql, entry.error].filter(Boolean).join('\n\n')
  return (
    <button type="button" className={styles.entry} title={tooltip} onClick={onClick}>
      <span
        className={`${styles.dot} ${failed ? styles.failed : styles.ok}`}
        aria-label={failed ? 'Failed' : 'Succeeded'}
      />
      <span className={styles.sql}>{firstLine(entry.sql)}</span>
      <span className={styles.time}>{formatTime(entry.executedAt)}</span>
      <span className={styles.meta}>
        {entry.connectionName}
        {entry.database && ` · ${entry.database}`} · {formatDuration(entry.elapsedMs)} ·{' '}
        <span className={failed ? styles.failedText : undefined}>{outcomeLabel(entry)}</span>
      </span>
    </button>
  )
}
