import type { PointerEvent } from 'react'
import styles from './ResizeHandle.module.css'
import { useMessages } from '../lib/i18n/useLanguage'
import { messages } from './messages'

interface ResizeHandleProps {
  /** The edge of the sized element the handle sits on; dragging it outward grows the element. */
  edge: 'left' | 'right' | 'top' | 'bottom'
  /**
   * Called while dragging with the new size (width for a side edge, height for
   * the top or bottom), from the size the element had when the drag began.
   */
  onResize: (size: number) => void
  /** A double click: back to the default size. */
  onReset?: () => void
  label: string
  className?: string
}

/** A thin strip on an element's edge: drag it to size the element. Its parent must be positioned. */
export function ResizeHandle({ edge, onResize, onReset, label, className }: ResizeHandleProps) {
  const m = useMessages(messages)
  const start = (event: PointerEvent<HTMLElement>) => {
    if (event.button !== 0) return
    event.preventDefault()
    const handle = event.currentTarget
    const vertical = edge === 'top' || edge === 'bottom'
    const position = (e: { clientX: number; clientY: number }) => (vertical ? e.clientY : e.clientX)
    const origin = position(event)
    const box = handle.parentElement!.getBoundingClientRect()
    const initial = vertical ? box.height : box.width
    const direction = edge === 'right' || edge === 'bottom' ? 1 : -1
    handle.setPointerCapture(event.pointerId)
    const move = (e: globalThis.PointerEvent) =>
      onResize(Math.round(initial + direction * (position(e) - origin)))
    const stop = () => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', stop)
      handle.removeEventListener('pointercancel', stop)
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', stop)
    handle.addEventListener('pointercancel', stop)
  }

  return (
    <span
      role="separator"
      aria-orientation={edge === 'top' || edge === 'bottom' ? 'horizontal' : 'vertical'}
      aria-label={label}
      title={onReset ? m.resetHint(label) : label}
      className={[styles.handle, styles[edge], className].filter(Boolean).join(' ')}
      onPointerDown={start}
      onDoubleClick={onReset}
    />
  )
}
