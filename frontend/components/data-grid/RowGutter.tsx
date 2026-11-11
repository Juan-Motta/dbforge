import type { CSSProperties, HTMLAttributes, ReactNode } from 'react'
import styles from './RowGutter.module.css'

interface RowSheetProps {
  /** The highest row number, so the column is as wide as it from the start. */
  lastNumber: number
  /** One `GutterNumber` per row drawn, and a `GutterGap` for each run left out, in the table's order. */
  numbers: ReactNode
  /** The table the numbers stand beside. */
  children: ReactNode
}

/**
 * A grid's table with its row numbers in a column of their own, which stays
 * at the left edge while the table scrolls across. Goes inside the scroller.
 */
export function RowSheet({ lastNumber, numbers, children }: RowSheetProps) {
  return (
    <div className={styles.sheet}>
      <div className={styles.gutter} style={{ width: `calc(${String(lastNumber).length}ch + 21px)` }}>
        <div className={styles.head}>#</div>
        {numbers}
      </div>
      {children}
    </div>
  )
}

/** Stands in for `rows` row numbers out of sight, each `rowHeight` pixels tall. */
export function GutterGap({ rows, rowHeight }: { rows: number; rowHeight: number }) {
  return (
    <div
      className={styles.gap}
      aria-hidden="true"
      style={{ height: rows * rowHeight, '--row-height': `${rowHeight}px` } as CSSProperties}
    />
  )
}

/** One row's number, as tall as the table's row. */
export function GutterNumber({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={className ? `${styles.number} ${className}` : styles.number} {...props} />
}
