import { useState } from 'react'
import { IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import type { SqlDialect } from '../../domain/connection'
import { filterLabels, isFiltered, NO_FILTER, type RowFilter } from './rowFilter'
import styles from './TableView.module.css'

interface FilterBarProps {
  dialect: SqlDialect
  /** The filter the rows are shown with. */
  value: RowFilter
  onApply: (filter: RowFilter) => void
  /** Pending edits would be hidden by another filter: apply or discard them first. */
  disabled: boolean
}

/** A `WHERE` and an `ORDER BY` (or a MongoDB filter and sort), applied with Enter. */
export function FilterBar({ dialect, value, onApply, disabled }: FilterBarProps) {
  const [draft, setDraft] = useState(value)
  const labels = filterLabels(dialect)
  // A header click changes the order from outside; the field follows it.
  const [seen, setSeen] = useState(value)
  if (seen !== value) {
    setSeen(value)
    setDraft(value)
  }
  const field = (key: keyof RowFilter, label: string, hint: string, grow: string) => (
    <label className={styles.filterField} style={{ flex: grow }}>
      <span className={styles.filterLabel}>{label}</span>
      <input
        value={draft[key]}
        placeholder={hint}
        spellCheck={false}
        disabled={disabled}
        aria-label={label}
        onChange={(event) => setDraft({ ...draft, [key]: event.target.value })}
        onKeyDown={(event) => {
          if (event.key === 'Enter') onApply(draft)
          if (event.key === 'Escape') setDraft(value)
        }}
      />
    </label>
  )

  return (
    <div
      className={styles.filterBar}
      title={disabled ? 'Apply or discard the pending changes first' : undefined}
    >
      {field('filter', labels.filter, labels.filterHint, '1 1 0')}
      {field('order', labels.order, labels.orderHint, '0 1 280px')}
      {/* Always there, so the fields keep their width; usable only while a filter applies. */}
      <IconButton
        label="Clear filter and order"
        disabled={disabled || !isFiltered(value)}
        onClick={() => onApply(NO_FILTER)}
      >
        <Icon name="close" />
      </IconButton>
    </div>
  )
}
