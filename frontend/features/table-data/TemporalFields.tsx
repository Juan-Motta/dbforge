import { useLanguage, useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import { daysInMonth, type Temporal, type TemporalKind } from './temporal'
import styles from './TemporalFields.module.css'

interface TemporalFieldsProps {
  kind: TemporalKind
  parts: Temporal
  onChange: (changes: Partial<Temporal>) => void
}

/** Day and month dropdowns and a year spinner for dates, hour : minute : second spinners for times. */
export function TemporalFields({ kind, parts, onChange }: TemporalFieldsProps) {
  const m = useMessages(messages)
  const { language } = useLanguage()
  const months = Array.from({ length: 12 }, (_, index) =>
    new Intl.DateTimeFormat(language, { month: 'long', timeZone: 'UTC' }).format(
      new Date(Date.UTC(2000, index, 1)),
    ),
  )

  return (
    <>
      {kind !== 'time' && (
        <div className={styles.row}>
          <select
            aria-label={m.day}
            value={parts.day}
            onChange={(e) => onChange({ day: Number(e.target.value) })}
          >
            {Array.from({ length: daysInMonth(parts.year, parts.month) }, (_, index) => (
              <option key={index} value={index + 1}>
                {index + 1}
              </option>
            ))}
          </select>
          <select
            aria-label={m.month}
            value={parts.month}
            onChange={(e) => onChange({ month: Number(e.target.value) })}
          >
            {months.map((name, index) => (
              <option key={name} value={index + 1}>
                {name}
              </option>
            ))}
          </select>
          <input
            type="number"
            aria-label={m.year}
            className={styles.year}
            min={1}
            max={9999}
            value={parts.year}
            onChange={(e) => {
              const year = Number(e.target.value)
              if (year >= 1 && year <= 9999) onChange({ year })
            }}
          />
        </div>
      )}
      {kind !== 'date' && (
        <div className={styles.row}>
          <TimeSpinner label={m.hour} value={parts.hour} max={23} onChange={(hour) => onChange({ hour })} />
          <span>:</span>
          <TimeSpinner
            label={m.minute}
            value={parts.minute}
            max={59}
            onChange={(minute) => onChange({ minute })}
          />
          <span>:</span>
          <TimeSpinner
            label={m.second}
            value={parts.second}
            max={59}
            onChange={(second) => onChange({ second })}
          />
          {parts.fraction && <span className={styles.note}>.{parts.fraction}</span>}
          {kind === 'timestamptz' && (
            <span className={styles.note} title={m.offsetKept}>
              UTC{formatOffset(parts.offset ?? 0)}
            </span>
          )}
        </div>
      )}
    </>
  )
}

function formatOffset(offset: number): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${offset < 0 ? '−' : '+'}${pad(Math.floor(Math.abs(offset) / 60))}:${pad(Math.abs(offset) % 60)}`
}

/** A two-digit spinner from 0 to `max`; a typed value past it wraps around (60 minutes is 00). */
function TimeSpinner({
  label,
  value,
  max,
  onChange,
}: {
  label: string
  value: number
  max: number
  onChange: (value: number) => void
}) {
  return (
    <input
      type="number"
      aria-label={label}
      className={styles.spinner}
      min={0}
      max={max}
      value={String(value).padStart(2, '0')}
      onChange={(event) => {
        const next = Number(event.target.value)
        if (Number.isInteger(next)) onChange(((next % (max + 1)) + max + 1) % (max + 1))
      }}
    />
  )
}
