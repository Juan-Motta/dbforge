import { useState } from 'react'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import styles from './ExportDialog.module.css'

interface ColumnPickerProps {
  /** Every column, in the table's order; `null` while loading, `undefined` when they can't be known. */
  columns: string[] | null | undefined
  chosen: string[]
  onChange: (chosen: string[]) => void
  disabled?: boolean
}

/** Which columns go in the export: all of them until some are left out. */
export function ColumnPicker({ columns, chosen, onChange, disabled }: ColumnPickerProps) {
  const m = useMessages(messages)
  const [open, setOpen] = useState(false)
  const known = columns ?? []

  const toggle = (name: string, on: boolean) =>
    onChange(on ? known.filter((c) => c === name || chosen.includes(c)) : chosen.filter((c) => c !== name))

  const summary =
    columns === null
      ? m.loadingColumns
      : columns === undefined
        ? m.columnsUnavailable
        : chosen.length === 0
          ? m.noColumnChosen
          : chosen.length === columns.length
            ? m.allColumns(columns.length)
            : m.someColumns(chosen.length, columns.length)

  return (
    <div className={styles.columns}>
      <div className={styles.columnRow}>
        <Icon name="column" size={15} />
        <span className={columns && chosen.length === 0 ? styles.error : styles.columnSummary}>
          {summary}
        </span>
        {open && (
          <>
            <button type="button" className={styles.link} disabled={disabled} onClick={() => onChange(known)}>
              {m.selectAll}
            </button>
            <button type="button" className={styles.link} disabled={disabled} onClick={() => onChange([])}>
              {m.selectNone}
            </button>
          </>
        )}
        {columns && (
          <Button onClick={() => setOpen(!open)} aria-expanded={open}>
            {open ? m.hideColumns : m.chooseColumns}
          </Button>
        )}
      </div>
      {open && columns && (
        <ul className={styles.columnList}>
          {columns.map((name, i) => (
            // A query result may have two columns of one name.
            <li key={`${i}:${name}`}>
              <label>
                <input
                  type="checkbox"
                  checked={chosen.includes(name)}
                  disabled={disabled}
                  onChange={(event) => toggle(name, event.target.checked)}
                />
                <span>{name}</span>
              </label>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
