import { useCallback, useRef, useState } from 'react'
import { copyText } from '../lib/clipboard'
import { useMessages } from '../lib/i18n/useLanguage'
import { Icon } from './Icon'
import { messages } from './messages'
import { useDismiss } from './useDismiss'
import styles from './ErrorLine.module.css'

/**
 * A failure on one line, cut with an ellipsis so the layout never moves; a
 * click opens all of it above the line, to read and copy.
 */
export function ErrorLine({ text }: { text: string }) {
  const m = useMessages(messages)
  const [open, setOpen] = useState(false)
  const [copied, setCopied] = useState(false)
  const root = useRef<HTMLSpanElement>(null)
  const close = useCallback(() => setOpen(false), [])
  useDismiss(open, root, close)

  return (
    <span className={styles.root} ref={root}>
      <button
        type="button"
        className={styles.line}
        aria-expanded={open}
        title={m.showError}
        onClick={() => {
          setCopied(false)
          setOpen((o) => !o)
        }}
      >
        {text}
      </button>
      {open && (
        <div className={styles.popover} role="dialog" aria-label={m.errorTitle}>
          <p className={styles.text}>{text}</p>
          <button
            type="button"
            className={styles.copy}
            onClick={() => {
              void copyText(text).then(() => setCopied(true))
            }}
          >
            <Icon name={copied ? 'check' : 'duplicate'} size={12} />
            {m.copy}
          </button>
        </div>
      )}
    </span>
  )
}
