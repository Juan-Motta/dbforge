import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Icon, type IconName } from './Icon'
import styles from './ContextMenu.module.css'

export interface MenuItem {
  label: string
  /** Leaves a submenu out; an item with `items` opens them instead. */
  onSelect?: () => void
  disabled?: boolean
  icon?: IconName
  /** A submenu, opened to the side on hover, click or →. */
  items?: MenuItem[]
}

interface ContextMenuProps {
  /** Where the pointer was, in viewport pixels. */
  x: number
  y: number
  items: MenuItem[]
  onClose: () => void
}

const MENU_WIDTH = 180
const ITEM_HEIGHT = 28

/** A small menu at the pointer. Escape, a click elsewhere or a choice closes it. */
export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const menu = useRef<HTMLDivElement>(null)

  useEffect(() => {
    menu.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
    const onKeyDown = (event: globalThis.KeyboardEvent) => event.key === 'Escape' && onClose()
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  const left = Math.min(x, window.innerWidth - MENU_WIDTH)
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
        className={styles.anchor}
        // Keep the menu inside the window near the right and bottom edges.
        style={{ left, top: Math.min(y, window.innerHeight - ITEM_HEIGHT * items.length - 8) }}
        onPointerDown={(event) => event.stopPropagation()}
      >
        <MenuList
          items={items}
          onClose={onClose}
          // Submenus open to the left when there is no room on the right.
          side={left + 2 * MENU_WIDTH > window.innerWidth ? 'left' : 'right'}
        />
      </div>
    </div>
  )
}

interface MenuListProps {
  items: MenuItem[]
  onClose: () => void
  side: 'left' | 'right'
  /** Set on a submenu: ← closes it and returns to its item. */
  onBack?: () => void
}

function MenuList({ items, onClose, side, onBack }: MenuListProps) {
  const [open, setOpen] = useState<number | null>(null)
  const list = useRef<HTMLDivElement>(null)

  /** The first enabled item of the submenu inside `entry`. */
  const focusSubmenu = (entry: HTMLElement) =>
    entry.querySelector<HTMLButtonElement>(':scope > div > [role=menu] button:not(:disabled)')?.focus()

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>, index: number, item: MenuItem) => {
    if (event.key === 'ArrowRight' && item.items) {
      event.preventDefault()
      event.stopPropagation()
      setOpen(index)
      const entry = event.currentTarget
      // The submenu renders on the next frame.
      requestAnimationFrame(() => focusSubmenu(entry))
    } else if (event.key === 'ArrowLeft' && onBack) {
      event.preventDefault()
      event.stopPropagation()
      onBack()
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      event.stopPropagation()
      const buttons = [...(list.current?.querySelectorAll<HTMLButtonElement>(':scope > div > button') ?? [])]
      const enabled = buttons.filter((button) => !button.disabled)
      const at = enabled.indexOf(document.activeElement as HTMLButtonElement)
      const step = event.key === 'ArrowDown' ? 1 : -1
      enabled[(at + step + enabled.length) % enabled.length]?.focus()
    }
  }

  return (
    <div ref={list} role="menu" className={styles.menu}>
      {items.map((item, index) => (
        <div
          key={item.label}
          className={styles.entry}
          onPointerEnter={() => setOpen(item.items ? index : null)}
          onKeyDown={(event) => onKeyDown(event, index, item)}
        >
          <button
            type="button"
            role="menuitem"
            aria-haspopup={item.items ? 'menu' : undefined}
            aria-expanded={item.items ? open === index : undefined}
            disabled={item.disabled}
            className={open === index ? styles.active : undefined}
            onClick={() => {
              if (item.items) {
                setOpen(open === index ? null : index)
                return
              }
              onClose()
              item.onSelect?.()
            }}
          >
            <span className={styles.icon}>{item.icon && <Icon name={item.icon} size={14} />}</span>
            <span className={styles.label}>{item.label}</span>
            {item.items && <Icon name="chevronRight" size={12} />}
          </button>
          {item.items && open === index && (
            <div className={side === 'right' ? styles.submenuRight : styles.submenuLeft}>
              <MenuList
                items={item.items}
                onClose={onClose}
                side={side}
                onBack={() => {
                  setOpen(null)
                  list.current?.querySelectorAll<HTMLButtonElement>(':scope > div > button')[index]?.focus()
                }}
              />
            </div>
          )}
        </div>
      ))}
    </div>
  )
}
