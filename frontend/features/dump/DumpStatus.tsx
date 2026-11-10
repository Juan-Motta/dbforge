import { useEffect, useState } from 'react'
import { Icon } from '../../components/Icon'
import type { Status } from './dumpJob'
import styles from './DumpDialogs.module.css'

/** Whole seconds since `startedAt`, as 0:07 or 1:12, ticking while mounted. */
function useElapsed(startedAt: number) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  const seconds = Math.max(0, Math.floor((now - startedAt) / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}

function Running({ title, line, startedAt }: { title: string; line: string; startedAt: number }) {
  const elapsed = useElapsed(startedAt)
  return (
    <div className={`${styles.callout} ${styles.running}`} role="status">
      <span className={styles.spinner} aria-hidden />
      <span className={styles.calloutText}>
        <span className={styles.calloutTitle}>
          {title}
          <span className={styles.elapsed}>{elapsed}</span>
        </span>
        <span className={`${styles.calloutDetail} ${styles.mono} ${styles.oneLine}`} title={line}>
          {line || ' '}
        </span>
      </span>
    </div>
  )
}

const ICONS = { done: 'check', warning: 'warning', failed: 'warning', stopped: 'stop' } as const

/**
 * The box under a dump form: the run in progress (with the tool's last line
 * and the time so far), then how it ended. Its space is kept while empty, so
 * the dialog doesn't jump when a run starts or ends.
 */
export function DumpStatus({
  running,
  runningTitle,
  line,
  startedAt,
  status,
}: {
  running: boolean
  runningTitle: string
  line: string
  startedAt: number
  status: Status
}) {
  if (running) return <Running title={runningTitle} line={line} startedAt={startedAt} />
  if (status.state === 'idle') return <div className={styles.callout} aria-hidden />
  return (
    <div
      className={`${styles.callout} ${styles[status.state]}`}
      role={status.state === 'failed' ? 'alert' : 'status'}
    >
      <Icon name={ICONS[status.state]} size={15} />
      <span className={styles.calloutText}>
        <span className={styles.calloutTitle}>{status.title}</span>
        {status.detail && (
          <span className={`${styles.calloutDetail} ${status.mono ? styles.mono : ''}`}>{status.detail}</span>
        )}
      </span>
    </div>
  )
}
