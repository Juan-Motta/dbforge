import type { PointerEvent } from 'react'
import styles from './ResizeHandle.module.css'

interface ResizeHandleProps {
  /** The edge of the sized element the handle sits on; dragging it outward widens the element. */
  edge: 'left' | 'right'
  /** Called while dragging with the new width, from the width the element had when the drag began. */
  onResize: (width: number) => void
  /** A double click: back to the default width. */
  onReset?: () => void
  label: string
  className?: string
}

/** A thin strip on an element's edge: drag it to size the element. Its parent must be positioned. */
export function ResizeHandle({ edge, onResize, onReset, label, className }: ResizeHandleProps) {
  const start = (event: PointerEvent<HTMLElement>) => {
    if (event.button !== 0) return
    event.preventDefault()
    const handle = event.currentTarget
    const origin = event.clientX
    const initial = handle.parentElement!.getBoundingClientRect().width
    const direction = edge === 'right' ? 1 : -1
    handle.setPointerCapture(event.pointerId)
    const move = (e: globalThis.PointerEvent) =>
      onResize(Math.round(initial + direction * (e.clientX - origin)))
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
      aria-orientation="vertical"
      aria-label={label}
      title={onReset ? `${label} · double-click to reset` : label}
      className={[styles.handle, styles[edge], className].filter(Boolean).join(' ')}
      onPointerDown={start}
      onDoubleClick={onReset}
    />
  )
}
