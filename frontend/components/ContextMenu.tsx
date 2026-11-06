import { useEffect, useRef } from 'react'
import styles from './ContextMenu.module.css'

export interface MenuItem {
  label: string
  onSelect: () => void
  disabled?: boolean
}

interface ContextMenuProps {
  /** Where the pointer was, in viewport pixels. */
  x: number
  y: number
  items: MenuItem[]
  onClose: () => void
}

/** A small menu at the pointer. Escape, a click elsewhere or a choice closes it. */
export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const menu = useRef<HTMLDivElement>(null)

  useEffect(() => {
    menu.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
    const onKeyDown = (event: KeyboardEvent) => event.key === 'Escape' && onClose()
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  return (
    <div
      className={styles.backdrop}
      onPointerDown={onClose}
      onContextMenu={(event) => {
        event.preventDefault()
        onClose()
      }}
    >
      <div
        ref={menu}
        role="menu"
        className={styles.menu}
        // Keep the menu inside the window near the right and bottom edges.
        style={{
          left: Math.min(x, window.innerWidth - 180),
          top: Math.min(y, window.innerHeight - 40 * items.length),
        }}
        onPointerDown={(event) => event.stopPropagation()}
      >
        {items.map((item) => (
          <button
            key={item.label}
            type="button"
            role="menuitem"
            disabled={item.disabled}
            onClick={() => {
              onClose()
              item.onSelect()
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
    </div>
  )
}
