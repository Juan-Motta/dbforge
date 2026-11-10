import { useEffect, useId, useRef, type ReactNode } from 'react'
import { IconButton } from './Button'
import { Icon } from './Icon'
import styles from './Modal.module.css'
import { useMessages } from '../lib/i18n/useLanguage'
import { messages } from './messages'

interface ModalProps {
  title: string
  onClose: () => void
  children: ReactNode
  className?: string
}

/**
 * Modal built on the native <dialog>, which already handles Escape,
 * focus trapping and the backdrop. Render it only while it should be open.
 * Mark the element that should receive focus with `data-autofocus`.
 */
export function Modal({ title, onClose, children, className }: ModalProps) {
  const ref = useRef<HTMLDialogElement>(null)
  const titleId = useId()
  const m = useMessages(messages)

  useEffect(() => {
    const dialog = ref.current
    if (!dialog) return
    dialog.showModal()
    // showModal() moves focus to the first focusable element; prefer the one marked for it.
    dialog.querySelector<HTMLElement>('[data-autofocus]')?.focus()
  }, [])

  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      className={`${styles.modal} ${className ?? ''}`}
      onCancel={(event) => {
        event.preventDefault()
        onClose()
      }}
    >
      <header className={styles.header}>
        <h2 id={titleId}>{title}</h2>
        <IconButton label={m.close} onClick={onClose}>
          <Icon name="close" />
        </IconButton>
      </header>
      {children}
    </dialog>
  )
}
