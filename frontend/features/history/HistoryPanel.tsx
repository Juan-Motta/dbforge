import { useState } from 'react'
import { IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import type { HistoryEntry } from './api'
import { firstLine, formatDuration, formatTime, groupByDay, outcomeLabel } from './format'
import { useHistory } from './useHistory'
import styles from './HistoryPanel.module.css'

interface HistoryPanelProps {
  /** Opens the entry's SQL in a new console, without running it. */
  onOpen: (entry: HistoryEntry) => void
  /** False once the entry's connection has been deleted. */
  canOpen: (entry: HistoryEntry) => boolean
}

/** Every query run from a console, newest first, grouped by day. */
export function HistoryPanel({ onOpen, canOpen }: HistoryPanelProps) {
  const [search, setSearch] = useState('')
  const [errorsOnly, setErrorsOnly] = useState(false)
  const { entries, loadedAt, error, loading, reload } = useHistory(search, errorsOnly)

  return (
    <aside className={`island ${styles.panel}`} aria-label="Query history">
      <div className={styles.header}>
        <span className={styles.title}>Query history</span>
        <IconButton label="Reload history" disabled={loading} onClick={reload}>
          <Icon name="refresh" />
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
                <Entry key={entry.id} entry={entry} available={canOpen(entry)} onOpen={() => onOpen(entry)} />
              ))}
            </section>
          ))
        )}
      </div>
    </aside>
  )
}

function Entry({
  entry,
  available,
  onOpen,
}: {
  entry: HistoryEntry
  available: boolean
  onOpen: () => void
}) {
  const failed = entry.error !== null
  const tooltip = [entry.sql, entry.error, available ? null : 'Its connection was deleted.']
    .filter(Boolean)
    .join('\n\n')
  return (
    <button type="button" className={styles.entry} disabled={!available} title={tooltip} onClick={onOpen}>
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
