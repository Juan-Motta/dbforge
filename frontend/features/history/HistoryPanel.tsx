import { useState, type MouseEvent } from 'react'
import { IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { useMessages } from '../../lib/i18n/useLanguage'
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
import { messages } from './messages'
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
  const m = useMessages(messages)
  const [search, setSearch] = useState('')
  const [errorsOnly, setErrorsOnly] = useState(false)
  const [scope, setScope] = useState<HistoryScope>('connection')
  const { entries: all, loadedAt, error, loading } = useHistory(search, errorsOnly, reloadKey)
  const entries = all && inScope(all, scope, connectionId)

  return (
    <aside className={styles.panel} aria-label={m.queryHistory}>
      <div className={styles.header}>
        <span className={styles.title}>{m.title}</span>
        <IconButton label={m.close} onClick={onClose}>
          <Icon name="close" />
        </IconButton>
      </div>

      <div className={styles.filters}>
        <label className={styles.search}>
          <Icon name="search" className={styles.searchIcon} />
          <input
            type="search"
            aria-label={m.searchSql}
            placeholder={m.searchSqlPlaceholder}
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
          {m.errors}
        </button>
      </div>
      <div className={styles.scope} role="group" aria-label={m.whoseRuns}>
        <button type="button" aria-pressed={scope === 'connection'} onClick={() => setScope('connection')}>
          {m.thisConnection}
        </button>
        <button type="button" aria-pressed={scope === 'all'} onClick={() => setScope('all')}>
          {m.all}
        </button>
      </div>

      <div className={styles.list} aria-busy={loading}>
        {error ? (
          <p className={styles.error}>{m.loadFailed(error)}</p>
        ) : entries?.length === 0 ? (
          <p className={styles.empty}>{search || errorsOnly ? m.noRunsMatch : m.emptyHistory}</p>
        ) : (
          entries &&
          groupByDay(entries, new Date(loadedAt), m).map((group) => (
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
      <p className={styles.hint}>{m.hint}</p>
    </aside>
  )
}

function Entry({ entry, onClick }: { entry: HistoryEntry; onClick: (event: MouseEvent) => void }) {
  const m = useMessages(messages)
  const failed = entry.error !== null
  const tooltip = [entry.sql, entry.error].filter(Boolean).join('\n\n')
  return (
    <button type="button" className={styles.entry} title={tooltip} onClick={onClick}>
      <span
        className={`${styles.dot} ${failed ? styles.failed : styles.ok}`}
        aria-label={failed ? m.failed : m.succeeded}
      />
      <span className={styles.sql}>{firstLine(entry.sql)}</span>
      <span className={styles.time}>{formatTime(entry.executedAt)}</span>
      <span className={styles.meta}>
        {entry.connectionName}
        {entry.database && ` · ${entry.database}`} · {formatDuration(entry.elapsedMs)} ·{' '}
        <span className={failed ? styles.failedText : undefined}>{outcomeLabel(entry, m)}</span>
      </span>
    </button>
  )
}
