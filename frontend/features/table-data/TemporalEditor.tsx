import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Button } from '../../components/Button'
import { useLanguage, useMessages } from '../../lib/i18n/useLanguage'
/** A cell's text; `null` is SQL NULL. */
type Value = string | null
import { messages } from './messages'
import {
  daysInMonth,
  formatTemporal,
  parseTemporal,
  temporalOf,
  withParts,
  type Temporal,
  type TemporalKind,
} from './temporal'
import styles from './EditableGrid.module.css'

interface TemporalEditorProps {
  initial: Value
  kind: TemporalKind
  /** SQL Server writes the offset after a space. */
  offsetSpace: boolean
  nullable: boolean
  onCommit: (value: Value) => void
  onCancel: () => void
}

/** Where the controls open, in window pixels: under the cell, or above it when there is no room. */
interface Placement {
  left: number
  top?: number
  bottom?: number
}

/** About the panel's size, to keep it inside the window. */
const PANEL_WIDTH = 340
const PANEL_HEIGHT = 170

function placeUnder(rect: DOMRect): Placement {
  const left = Math.max(8, Math.min(rect.left, window.innerWidth - PANEL_WIDTH - 8))
  return window.innerHeight - rect.bottom >= PANEL_HEIGHT
    ? { left, top: rect.bottom + 2 }
    : { left, bottom: window.innerHeight - rect.top + 2 }
}

/**
 * A date or time cell: its text, typed as before, and under it the parts as
 * controls (day and month dropdowns, year and time spinners), Now or Today,
 * and NULL. Enter or Apply keeps the value; Escape, Cancel or a click outside
 * without changes leaves the cell as it was.
 */
export function TemporalEditor({
  initial,
  kind,
  offsetSpace,
  nullable,
  onCommit,
  onCancel,
}: TemporalEditorProps) {
  const m = useMessages(messages)
  const { language } = useLanguage()
  const [text, setText] = useState(initial ?? '')
  // The controls show the last valid value; a NULL cell starts them at now.
  const [parts, setParts] = useState<Temporal>(
    () => parseTemporal(initial ?? '', kind) ?? temporalOf(new Date()),
  )
  const [placement, setPlacement] = useState<Placement | null>(null)
  const root = useRef<HTMLSpanElement>(null)
  const panel = useRef<HTMLDivElement>(null)

  const valid = text === '' || parseTemporal(text, kind) !== null
  const hasDate = kind !== 'time'
  const hasTime = kind !== 'date'

  const finish = () => {
    if (text === (initial ?? '')) return onCancel()
    const parsed = parseTemporal(text, kind)
    // Written back in the server's own form, whatever spacing was typed.
    if (parsed) onCommit(formatTemporal(parsed, kind, offsetSpace))
  }

  const change = (changes: Partial<Temporal>) => {
    const next = withParts(parts, changes)
    setParts(next)
    setText(formatTemporal(next, kind, offsetSpace))
  }

  const now = () => {
    const current = temporalOf(new Date())
    // Without a time zone the value stays as written; keep the column's own offset otherwise.
    const next = kind === 'timestamptz' ? current : { ...current, offset: parts.offset }
    setParts(next)
    setText(formatTemporal(next, kind, offsetSpace))
  }

  // A click outside keeps the value, as leaving the plain editor does.
  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node
      if (!root.current?.contains(target) && !panel.current?.contains(target)) finish()
    }
    document.addEventListener('pointerdown', onPointerDown)
    return () => document.removeEventListener('pointerdown', onPointerDown)
  })

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Enter') {
      event.preventDefault()
      finish()
    } else if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      onCancel()
    }
  }

  const months = Array.from({ length: 12 }, (_, index) =>
    new Intl.DateTimeFormat(language, { month: 'long', timeZone: 'UTC' }).format(
      new Date(Date.UTC(2000, index, 1)),
    ),
  )
  const number = (value: number, width: number) => String(value).padStart(width, '0')

  return (
    <span
      ref={(element) => {
        root.current = element
        if (element && !placement) setPlacement(placeUnder(element.getBoundingClientRect()))
      }}
      className={styles.editor}
      onKeyDown={onKeyDown}
    >
      <input
        autoFocus
        onFocus={(event) => event.target.select()}
        aria-invalid={!valid}
        value={text}
        onChange={(event) => {
          setText(event.target.value)
          const parsed = parseTemporal(event.target.value, kind)
          if (parsed) setParts(parsed)
        }}
      />
      {placement && (
        <div
          ref={panel}
          className={styles.temporal}
          style={placement}
          role="group"
          aria-label={m.dateTime}
          onKeyDown={onKeyDown}
        >
          {hasDate && (
            <div className={styles.temporalRow}>
              <select
                aria-label={m.day}
                value={parts.day}
                onChange={(e) => change({ day: Number(e.target.value) })}
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
                onChange={(e) => change({ month: Number(e.target.value) })}
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
                  if (year >= 1 && year <= 9999) change({ year })
                }}
              />
            </div>
          )}
          {hasTime && (
            <div className={styles.temporalRow}>
              <TimeSpinner label={m.hour} value={parts.hour} max={23} onChange={(hour) => change({ hour })} />
              <span>:</span>
              <TimeSpinner
                label={m.minute}
                value={parts.minute}
                max={59}
                onChange={(minute) => change({ minute })}
              />
              <span>:</span>
              <TimeSpinner
                label={m.second}
                value={parts.second}
                max={59}
                onChange={(second) => change({ second })}
              />
              {parts.fraction && <span className={styles.temporalNote}>.{parts.fraction}</span>}
              {kind === 'timestamptz' && (
                <span className={styles.temporalNote} title={m.offsetKept}>
                  UTC{formatOffset(parts.offset ?? 0, number)}
                </span>
              )}
            </div>
          )}
          <div className={styles.temporalActions}>
            <Button onClick={now}>{kind === 'date' ? m.today : m.now}</Button>
            {nullable && <Button onClick={() => onCommit(null)}>NULL</Button>}
            <span className={styles.temporalSpacer} />
            <Button variant="ghost" onClick={onCancel}>
              {m.cancel}
            </Button>
            <Button variant="primary" disabled={!valid} onClick={finish}>
              {m.apply}
            </Button>
          </div>
        </div>
      )}
    </span>
  )
}

function formatOffset(offset: number, number: (value: number, width: number) => string): string {
  const sign = offset < 0 ? '−' : '+'
  return `${sign}${number(Math.floor(Math.abs(offset) / 60), 2)}:${number(Math.abs(offset) % 60, 2)}`
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
