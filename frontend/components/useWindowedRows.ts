import { useCallback, useState, type UIEvent } from 'react'

export interface RowWindow {
  /** The first row to draw, and the one after the last. */
  start: number
  end: number
  /** The height of the rows left out before and after, for the spacers that stand in for them. */
  before: number
  after: number
}

/**
 * The rows of a list of `count` rows of one height that are in sight, with
 * `overscan` more on each side so a scroll shows rows before the next draw.
 */
export function rowWindow(
  scrollTop: number,
  viewHeight: number,
  rowHeight: number,
  count: number,
  overscan: number,
): RowWindow {
  const start = Math.min(count, Math.max(0, Math.floor(scrollTop / rowHeight) - overscan))
  const end = Math.min(count, Math.max(start, Math.ceil((scrollTop + viewHeight) / rowHeight) + overscan))
  return { start, end, before: start * rowHeight, after: (count - end) * rowHeight }
}

/** Rows drawn beyond the ones in sight, on each side. */
const OVERSCAN = 12
/** The window moves in steps of this many rows, so most scroll events draw nothing. */
const STEP = 4

/**
 * Draws only the rows in sight of a long list whose rows share one height.
 * `ref` and `onScroll` go on the scrolling element; the rows from `start` to
 * `end` go between two spacers of `before` and `after` pixels.
 */
export function useWindowedRows(count: number, rowHeight: number) {
  const [view, setView] = useState({ top: 0, height: 0 })

  const ref = useCallback((element: HTMLElement | null) => {
    if (!element) return
    const measure = () =>
      setView((current) =>
        current.height === element.clientHeight ? current : { ...current, height: element.clientHeight },
      )
    measure()
    // Also when a hidden tab is shown: it has no height until then.
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const onScroll = useCallback(
    (event: UIEvent<HTMLElement>) => {
      const step = rowHeight * STEP
      const top = Math.floor(event.currentTarget.scrollTop / step) * step
      setView((current) => (current.top === top ? current : { ...current, top }))
    },
    [rowHeight],
  )

  // A step of rows more below, since `top` is rounded down to a step.
  const rows = rowWindow(view.top, view.height + rowHeight * STEP, rowHeight, count, OVERSCAN)
  return { ref, onScroll, ...rows }
}
