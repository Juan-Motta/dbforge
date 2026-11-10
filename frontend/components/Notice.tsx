import styles from './Notice.module.css'

export interface NoticeState {
  /** Changes with every notice, so the same text shown twice still restarts. */
  id: number
  text: string
  tone: 'info' | 'error'
}

/** A short line at the bottom of the window, e.g. "Copied"; the owner hides it after a moment. */
export function Notice({ notice }: { notice: NoticeState }) {
  return (
    <div role="status" className={`${styles.notice} ${notice.tone === 'error' ? styles.error : ''}`}>
      {notice.text}
    </div>
  )
}
