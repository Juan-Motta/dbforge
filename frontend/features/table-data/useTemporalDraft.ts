import { useState } from 'react'
import {
  formatTemporal,
  parseTemporal,
  temporalOf,
  withParts,
  type Temporal,
  type TemporalKind,
} from './temporal'

/**
 * A date or time being edited: the typed text and the parts the controls
 * show, kept in step. A NULL value starts the controls at now.
 */
export function useTemporalDraft(initial: string | null, kind: TemporalKind, offsetSpace: boolean) {
  const [text, setText] = useState(initial ?? '')
  // The controls show the last valid value.
  const [parts, setParts] = useState<Temporal>(
    () => parseTemporal(initial ?? '', kind) ?? temporalOf(new Date()),
  )

  const show = (next: Temporal) => {
    setParts(next)
    setText(formatTemporal(next, kind, offsetSpace))
  }

  return {
    text,
    parts,
    /** Empty, or a valid value of the kind. */
    valid: text === '' || parseTemporal(text, kind) !== null,
    /** The value in the server's own form; `null` while the text isn't one. */
    value: (() => {
      const parsed = parseTemporal(text, kind)
      return parsed && formatTemporal(parsed, kind, offsetSpace)
    })(),
    type: (typed: string) => {
      setText(typed)
      const parsed = parseTemporal(typed, kind)
      if (parsed) setParts(parsed)
    },
    change: (changes: Partial<Temporal>) => show(withParts(parts, changes)),
    /** This computer's clock; without a time zone the value keeps the column's own offset. */
    now: () => {
      const current = temporalOf(new Date())
      show(kind === 'timestamptz' ? current : { ...current, offset: parts.offset })
    },
  }
}
