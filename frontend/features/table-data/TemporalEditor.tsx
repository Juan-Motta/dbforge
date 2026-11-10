import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Button } from '../../components/Button'
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import type { TemporalKind } from './temporal'
import { TemporalFields } from './TemporalFields'
import { useTemporalDraft } from './useTemporalDraft'
import styles from './EditableGrid.module.css'

/** A cell's text; `null` is SQL NULL. */
type Value = string | null

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
  const draft = useTemporalDraft(initial, kind, offsetSpace)
  const [placement, setPlacement] = useState<Placement | null>(null)
  const root = useRef<HTMLSpanElement>(null)
  const panel = useRef<HTMLDivElement>(null)

  const finish = () => {
    if (draft.text === (initial ?? '')) return onCancel()
    // Written back in the server's own form, whatever spacing was typed.
    if (draft.value) onCommit(draft.value)
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
        aria-invalid={!draft.valid}
        value={draft.text}
        onChange={(event) => draft.type(event.target.value)}
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
          <TemporalFields kind={kind} parts={draft.parts} onChange={draft.change} />
          <div className={styles.temporalActions}>
            <Button onClick={draft.now}>{kind === 'date' ? m.today : m.now}</Button>
            {nullable && <Button onClick={() => onCommit(null)}>NULL</Button>}
            <span className={styles.temporalSpacer} />
            <Button variant="ghost" onClick={onCancel}>
              {m.cancel}
            </Button>
            <Button variant="primary" disabled={!draft.valid} onClick={finish}>
              {m.apply}
            </Button>
          </div>
        </div>
      )}
    </span>
  )
}
