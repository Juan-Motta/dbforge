import type { ReactNode } from 'react'
import styles from './Badge.module.css'

export type Tone = 'ok' | 'warn' | 'err' | 'info' | 'muted' | 'accent'

export function Badge({ tone = 'muted', children }: { tone?: Tone; children: ReactNode }) {
  return <span className={`${styles.badge} ${styles[tone]}`}>{children}</span>
}
