import { Select } from '../../components/Select'
import { TextField } from '../../components/TextField'
import { useMessages } from '../../lib/i18n/useLanguage'
import { ROWS_PER_INSERT, type ExportFormat, type FormatOptions, type JsonStyle } from './formats'
import { messages } from './messages'
import styles from './ExportDialog.module.css'

interface FormatOptionsFieldsProps {
  format: ExportFormat
  options: FormatOptions
  onChange: (options: FormatOptions) => void
}

/**
 * The options of the chosen format, in one row of the same height for every
 * format, so the dialog keeps its size when the format changes.
 */
export function FormatOptionsFields({ format, options, onChange }: FormatOptionsFieldsProps) {
  const m = useMessages(messages)
  const set = (change: Partial<FormatOptions>) => onChange({ ...options, ...change })

  switch (format) {
    case 'csv':
    case 'tsv':
      return (
        <div className={styles.options}>
          <Select
            label={m.header}
            value={options.header ? 'include' : 'leaveOut'}
            options={[
              { value: 'include', label: m.headerInclude },
              { value: 'leaveOut', label: m.headerLeaveOut },
            ]}
            onChange={(value) => set({ header: value === 'include' })}
          />
          <TextField
            label={m.nullText}
            mono
            value={options.nullText}
            placeholder={m.nullTextPlaceholder}
            onChange={(event) => set({ nullText: event.target.value })}
          />
        </div>
      )
    case 'json':
      return (
        <div className={styles.options}>
          <Select<JsonStyle>
            label={m.jsonLayout}
            value={options.jsonStyle}
            options={[
              { value: 'array', label: m.jsonArray },
              { value: 'lines', label: m.jsonLines },
            ]}
            onChange={(jsonStyle) => set({ jsonStyle })}
          />
        </div>
      )
    case 'sql':
      return (
        <div className={styles.options}>
          <Select
            label={m.rowsPerInsert}
            value={String(options.rowsPerInsert)}
            options={ROWS_PER_INSERT.map((count) => ({
              value: String(count),
              label: m.rowsPerInsertOption(count),
            }))}
            onChange={(value) => set({ rowsPerInsert: Number(value) })}
          />
        </div>
      )
    case 'markdown':
      return (
        <div className={styles.options}>
          <p className={styles.noOptions}>{m.noOptions}</p>
        </div>
      )
  }
}
