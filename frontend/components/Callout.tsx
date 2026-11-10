import type { ReactNode } from 'react'
import { Icon, type IconName } from './Icon'
import styles from './Callout.module.css'

export type CalloutTone = 'progress' | 'done' | 'warning' | 'failed' | 'stopped'

const ICONS: Record<Exclude<CalloutTone, 'progress'>, IconName> = {
  done: 'check',
  warning: 'warning',
  failed: 'warning',
  stopped: 'stop',
}

interface CalloutProps {
  tone: CalloutTone
  title: ReactNode
  detail?: ReactNode
  /** The detail is a tool's output or a path, in monospace. */
  mono?: boolean
  /** The detail stays on one line, cut with an ellipsis: a line that keeps changing. */
  oneLine?: boolean
  /** Shown at the end of the title, e.g. the time so far. */
  aside?: ReactNode
}

/**
 * A box saying how something is going or how it went: a spinner while in
 * progress, then an icon and color for done, warning, failed or stopped,
 * with a title and an optional detail.
 */
export function Callout({ tone, title, detail, mono, oneLine, aside }: CalloutProps) {
  const detailClass = [styles.detail, mono && styles.mono, oneLine && styles.oneLine]
    .filter(Boolean)
    .join(' ')
  return (
    <div className={`${styles.callout} ${styles[tone]}`} role={tone === 'failed' ? 'alert' : 'status'}>
      {tone === 'progress' ? (
        <span className={styles.spinner} aria-hidden />
      ) : (
        <Icon name={ICONS[tone]} size={15} />
      )}
      <span className={styles.text}>
        <span className={styles.title}>
          {title}
          {aside !== undefined && <span className={styles.aside}>{aside}</span>}
        </span>
        {detail !== undefined && (
          <span className={detailClass} title={oneLine && typeof detail === 'string' ? detail : undefined}>
            {detail}
          </span>
        )}
      </span>
    </div>
  )
}

/** The space a Callout takes, kept while there is nothing to say so the layout doesn't jump. */
export function CalloutSpace() {
  return <div className={styles.callout} aria-hidden />
}
