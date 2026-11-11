import { memo, useCallback, useLayoutEffect, useRef, useState } from 'react'
import { Badge, type Tone } from '../../components/Badge'
import { Button, IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { ResizeHandle } from '../../components/ResizeHandle'
import { usePanelSize } from '../../components/usePanelSize'
import type { Engine } from '../../domain/connection'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { Origin, StatementEntry } from './api'
import { useStatementLog } from './useStatementLog'
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
  import: 'originImport',
  dump: 'originDump',
  connect: 'originConnect',
  agent: 'originAgent',
}

interface StatementConsoleProps {
  connectionOf: (id: string) => { name: string; engine: Engine } | undefined
  /** The same function between renders, so the rows aren't drawn again for it. */
  onCopy: (text: string) => void
  onClose: () => void
}

/**
 * The statements DBForge sends to the databases, newest at the bottom, as an
 * island under the workspace: each one as it could be run again, with where
 * it ran, what it was for, how long it took and what came back.
 */
export function StatementConsole({ connectionOf, onCopy, onClose }: StatementConsoleProps) {
  const m = useMessages(messages)
  const { entries, clear } = useStatementLog()
  const [height, setHeight, resetHeight] = usePanelSize('dbforge.statementsHeight', 220, 120, 640)
  const [kind, setKind] = useState<Kind>('all')
  const [expanded, setExpanded] = useState<number | null>(null)
  const toggle = useCallback((id: number) => setExpanded((open) => (open === id ? null : id)), [])
  const list = useRef<HTMLDivElement>(null)
  // Follows new statements while scrolled to the bottom; stays put once scrolled up.
  const following = useRef(true)
  const shown = visible(entries, kind)
  const engineOf = (id: string) => connectionOf(id)?.engine

  // By the last statement, not by how many there are: a full log drops one for each it takes.
  const newest = shown.at(-1)?.id
  useLayoutEffect(() => {
    if (following.current && list.current) list.current.scrollTop = list.current.scrollHeight
  }, [newest, kind])

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
        <Button variant="ghost" disabled={entries.length === 0} onClick={clear}>
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
          shown.map((entry) => (
            <StatementRow
              key={entry.id}
              entry={entry}
              connection={connectionOf(entry.connectionId)}
              open={expanded === entry.id}
              onToggle={toggle}
              onCopy={onCopy}
            />
          ))
        )}
      </div>
    </section>
  )
}

interface StatementRowProps {
  entry: StatementEntry
  connection: { name: string; engine: Engine } | undefined
  open: boolean
  onToggle: (id: number) => void
  onCopy: (text: string) => void
}

/** One statement. Drawn again only when it changes: a new statement leaves the others as they are. */
const StatementRow = memo(function StatementRow({
  entry,
  connection,
  open,
  onToggle,
  onCopy,
}: StatementRowProps) {
  const m = useMessages(messages)
  const where = [connection?.name ?? m.unknownConnection, entry.database].filter(Boolean).join(' · ')
  return (
    <div className={styles.row} data-error={entry.error !== null || undefined}>
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
        onClick={() => onToggle(entry.id)}
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
