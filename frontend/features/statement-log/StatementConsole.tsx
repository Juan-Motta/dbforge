import { useLayoutEffect, useRef, useState } from 'react'
import { Badge, type Tone } from '../../components/Badge'
import { Button, IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { ResizeHandle } from '../../components/ResizeHandle'
import { usePanelSize } from '../../components/usePanelSize'
import type { Engine } from '../../domain/connection'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { Origin, StatementEntry } from './api'
import { messages } from './messages'
import { clockTime, duration, KINDS, kindOf, type Kind, runnable, script, visible } from './statementLog'
import styles from './StatementConsole.module.css'

/** What a statement was for, at a glance: user's own, changes, reads, metadata. */
const KIND_TONES: Record<Exclude<Kind, 'all'>, Tone> = {
  console: 'info',
  agents: 'accent',
  changes: 'warn',
  reads: 'ok',
  metadata: 'muted',
}

const ORIGIN_LABELS: Record<Origin, `origin${Capitalize<Origin>}`> = {
  console: 'originConsole',
  browse: 'originBrowse',
  edit: 'originEdit',
  metadata: 'originMetadata',
  admin: 'originAdmin',
  export: 'originExport',
  dump: 'originDump',
  connect: 'originConnect',
  agent: 'originAgent',
}

interface StatementConsoleProps {
  entries: StatementEntry[]
  connectionOf: (id: string) => { name: string; engine: Engine } | undefined
  onCopy: (text: string) => void
  onClear: () => void
  onClose: () => void
}

/**
 * The statements DBForge sends to the databases, newest at the bottom, as an
 * island under the workspace: each one as it could be run again, with where
 * it ran, what it was for, how long it took and what came back.
 */
export function StatementConsole({ entries, connectionOf, onCopy, onClear, onClose }: StatementConsoleProps) {
  const m = useMessages(messages)
  const [height, setHeight, resetHeight] = usePanelSize('dbforge.statementsHeight', 220, 120, 640)
  const [kind, setKind] = useState<Kind>('all')
  const [expanded, setExpanded] = useState<number | null>(null)
  const list = useRef<HTMLDivElement>(null)
  // Follows new statements while scrolled to the bottom; stays put once scrolled up.
  const following = useRef(true)
  const shown = visible(entries, kind)
  const engineOf = (id: string) => connectionOf(id)?.engine

  useLayoutEffect(() => {
    if (following.current && list.current) list.current.scrollTop = list.current.scrollHeight
  }, [shown.length, kind])

  return (
    <section className={`island ${styles.console}`} style={{ height }} aria-label={m.title}>
      <ResizeHandle edge="top" label={m.resize} onResize={setHeight} onReset={resetHeight} />
      <header className={styles.toolbar}>
        <span className={styles.title}>{m.title}</span>
        <span className={styles.count}>{m.count(shown.length)}</span>
        <div role="radiogroup" aria-label={m.filter} className={styles.kinds}>
          {KINDS.map((value) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={kind === value}
              onClick={() => setKind(value)}
            >
              {m[value]}
            </button>
          ))}
        </div>
        <span className={styles.spacer} />
        <Button variant="ghost" disabled={shown.length === 0} onClick={() => onCopy(script(shown, engineOf))}>
          <Icon name="duplicate" />
          {m.copyAll}
        </Button>
        <Button variant="ghost" disabled={entries.length === 0} onClick={onClear}>
          <Icon name="trash" />
          {m.clear}
        </Button>
        <IconButton label={m.close} onClick={onClose}>
          <Icon name="close" />
        </IconButton>
      </header>

      <div
        ref={list}
        className={styles.list}
        onScroll={(event) => {
          const element = event.currentTarget
          following.current = element.scrollHeight - element.scrollTop - element.clientHeight < 24
        }}
      >
        {shown.length === 0 ? (
          <p className={styles.empty}>{entries.length === 0 ? m.empty : m.emptyFiltered}</p>
        ) : (
          shown.map((entry) => {
            const connection = connectionOf(entry.connectionId)
            const open = expanded === entry.id
            const where = [connection?.name ?? m.unknownConnection, entry.database]
              .filter(Boolean)
              .join(' · ')
            return (
              <div key={entry.id} className={styles.row} data-error={entry.error !== null || undefined}>
                <span className={styles.time} title={where}>
                  {clockTime(entry.at)}
                </span>
                <span className={styles.origin}>
                  <Badge tone={KIND_TONES[kindOf(entry.origin)]}>{m[ORIGIN_LABELS[entry.origin]]}</Badge>
                </span>
                <span className={styles.where} title={entry.database ?? undefined}>
                  {connection?.name ?? m.unknownConnection}
                  {entry.database && <span className={styles.database}> · {entry.database}</span>}
                </span>
                <button
                  type="button"
                  className={open ? `${styles.text} ${styles.open}` : styles.text}
                  aria-expanded={open}
                  onClick={() => setExpanded(open ? null : entry.id)}
                >
                  {entry.text}
                </button>
                <span className={styles.meta}>
                  {duration(entry.elapsedMs)}
                  {entry.error === null && entry.rows !== null && <> · {m.rows(entry.rows)}</>}
                </span>
                <IconButton
                  label={m.copy}
                  className={styles.copy}
                  onClick={() => onCopy(runnable(entry, connection?.engine))}
                >
                  <Icon name="duplicate" />
                </IconButton>
                {entry.error !== null && <p className={styles.error}>{entry.error}</p>}
              </div>
            )
          })
        )}
      </div>
    </section>
  )
}
