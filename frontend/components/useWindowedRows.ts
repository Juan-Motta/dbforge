import { useCallback, useState, type UIEvent } from 'react'
import { flushSync } from 'react-dom'

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

export type DrawnRow = { index: number } | { gap: number; from: number }

/**
 * The rows to draw, by position: those from `start` to `end`, and `kept` (a
 * row being edited) wherever it is, with a gap for each run of rows left out.
 */
export function drawnRows(start: number, end: number, kept: number | null, count: number): DrawnRow[] {
  const indexes = Array.from({ length: end - start }, (_, i) => start + i)
  if (kept !== null && kept >= 0 && (kept < start || kept >= end)) {
    if (kept < start) indexes.unshift(kept)
    else indexes.push(kept)
  }
  const parts: DrawnRow[] = []
  let next = 0
  for (const index of indexes) {
    if (index > next) parts.push({ gap: index - next, from: next })
    parts.push({ index })
    next = index + 1
  }
  if (next < count) parts.push({ gap: count - next, from: next })
  return parts
}

/** The fewest rows drawn beyond the ones in sight, on each side, while the list has no height yet. */
const MIN_OVERSCAN = 12

/**
 * How far the window reaches past the rows in sight, and how far a scroll
 * must go before it moves, in rows, for a view `viewHeight` pixels tall.
 * The page scrolls on its own thread and a fast scroll runs a few frames
 * ahead of the code that draws, so two views are drawn on each side; moving
 * every half view keeps at least a view and a half ahead, and draws rarely.
 */
export function windowReach(viewHeight: number, rowHeight: number) {
  const inSight = Math.ceil(viewHeight / rowHeight)
  return { overscan: Math.max(MIN_OVERSCAN, 2 * inSight), step: Math.max(1, Math.floor(inSight / 2)) }
}

/**
 * Lists of up to this many cells are drawn whole. A fast scroll can cross a
 * thousand rows before any code runs, so only a whole list never shows a gap;
 * past this size, drawing it all would cost more than the gap.
 */
const DRAW_WHOLE_CELLS = 25_000

/** Whether a list of `count` rows of `cellsPerRow` cells is drawn whole. */
export function drawsWhole(count: number, cellsPerRow: number): boolean {
  return count * cellsPerRow <= DRAW_WHOLE_CELLS
}

/**
 * Draws a list whose rows share one height: whole when it has up to
 * `DRAW_WHOLE_CELLS` cells, otherwise only the rows in sight.
 * `ref` and `onScroll` go on the scrolling element; the rows from `start` to
 * `end` go between two spacers of `before` and `after` pixels. `viewHeight`
 * is 0 until that element is shown and measured.
 */
export function useWindowedRows(count: number, rowHeight: number, cellsPerRow = 1) {
  const [view, setView] = useState({ top: 0, height: 0 })
  const whole = drawsWhole(count, cellsPerRow)
  const { overscan, step } = windowReach(view.height, rowHeight)

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
      if (whole) return
      const stride = rowHeight * step
      const top = Math.floor(event.currentTarget.scrollTop / stride) * stride
      if (top === view.top) return
      // Drawn before the next paint: React would otherwise leave a scroll
      // update for a later frame, and the spacer would show for one.
      flushSync(() => setView((current) => ({ ...current, top })))
    },
    [whole, rowHeight, step, view.top],
  )

  const rows = whole
    ? { start: 0, end: count, before: 0, after: 0 }
    : // A step of rows more below, since `top` is rounded down to a step.
      rowWindow(view.top, view.height + rowHeight * step, rowHeight, count, overscan)
  return { ref, onScroll, viewHeight: view.height, ...rows }
}
