import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Icon } from '../../components/Icon'
import { matchTyped, step } from '../../components/selectKeys'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { Choice } from './choices'
import { messages } from './messages'
import styles from './EditableGrid.module.css'

/** A cell's text; `null` is SQL NULL. */
type Value = string | null

interface ChoiceEditorProps {
  initial: Value
  choices: Choice[]
  nullable: boolean
  onCommit: (value: Value) => void
  onCancel: () => void
}

/** Where the list opens, in window pixels: under the cell, or above it when there is no room. */
interface Placement {
  left: number
  minWidth: number
  top?: number
  bottom?: number
}

const ROW_HEIGHT = 28

/**
 * A cell that takes one of a few values (a boolean, an enum): the list of
 * them under the cell, the current one highlighted. A click or Enter picks;
 * Escape or a click outside leaves the cell as it was.
 */
export function ChoiceEditor({ initial, choices, nullable, onCommit, onCancel }: ChoiceEditorProps) {
  const m = useMessages(messages)
  // NULL is the last option where the column allows it.
  const options: { value: Value; label: string }[] = [
    ...choices,
    ...(nullable ? [{ value: null, label: 'NULL' }] : []),
  ]
  const [active, setActive] = useState(() =>
    Math.max(
      0,
      options.findIndex((option) => option.value === initial),
    ),
  )
  const [placement, setPlacement] = useState<Placement | null>(null)
  const root = useRef<HTMLSpanElement>(null)
  const list = useRef<HTMLUListElement>(null)
  const typed = useRef({ text: '', at: 0 })

  const pick = (index: number) => {
    const option = options[index]
    if (!option || option.value === initial) onCancel()
    else onCommit(option.value)
  }

  // A click outside changes nothing.
  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node
      if (!root.current?.contains(target) && !list.current?.contains(target)) onCancel()
    }
    document.addEventListener('pointerdown', onPointerDown)
    return () => document.removeEventListener('pointerdown', onPointerDown)
  }, [onCancel])

  const onKeyDown = (event: KeyboardEvent) => {
    const keys: Record<string, () => void> = {
      ArrowDown: () => setActive((current) => step(current, 1, options.length)),
      ArrowUp: () => setActive((current) => step(current, -1, options.length)),
      Home: () => setActive(0),
      End: () => setActive(options.length - 1),
      Enter: () => pick(active),
      ' ': () => pick(active),
      Escape: onCancel,
      Tab: onCancel,
    }
    const run = keys[event.key]
    if (run) {
      event.preventDefault()
      event.stopPropagation()
      run()
      return
    }
    if (event.key.length === 1 && !event.metaKey && !event.ctrlKey) {
      const now = Date.now()
      typed.current = {
        text: now - typed.current.at < 700 ? typed.current.text + event.key : event.key,
        at: now,
      }
      const from = typed.current.text.length > 1 ? active : (active + 1) % options.length
      const match = matchTyped(
        options.map((option) => option.label),
        typed.current.text,
        from,
      )
      if (match >= 0) setActive(match)
    }
  }

  return (
    <span
      ref={(element) => {
        root.current = element
        if (element && !placement) {
          const rect = element.getBoundingClientRect()
          const height = Math.min(320, options.length * ROW_HEIGHT + 10)
          const left = Math.max(8, Math.min(rect.left, window.innerWidth - Math.max(rect.width, 160) - 8))
          setPlacement(
            window.innerHeight - rect.bottom >= height
              ? { left, minWidth: rect.width, top: rect.bottom + 2 }
              : { left, minWidth: rect.width, bottom: window.innerHeight - rect.top + 2 },
          )
        }
      }}
      className={`${styles.editor} ${styles.choiceCell}`}
    >
      {initial ?? 'NULL'}
      {placement && (
        <ul
          ref={(element) => {
            list.current = element
            element?.focus()
          }}
          role="listbox"
          aria-label={m.pickValue}
          aria-activedescendant={`choice-${active}`}
          tabIndex={-1}
          className={styles.choices}
          style={{ ...placement, maxHeight: 320 }}
          onKeyDown={onKeyDown}
        >
          {options.map((option, index) => (
            <li
              key={option.value ?? '\u0000null'}
              id={`choice-${index}`}
              role="option"
              aria-selected={option.value === initial}
              className={[
                styles.choice,
                index === active && styles.choiceActive,
                option.value === null && styles.null,
              ]
                .filter(Boolean)
                .join(' ')}
              ref={(element) => {
                if (index === active) element?.scrollIntoView({ block: 'nearest' })
              }}
              onPointerMove={() => setActive(index)}
              onClick={() => pick(index)}
            >
              <span className={styles.choiceLabel}>{option.label}</span>
              {option.value !== option.label && option.value !== null && (
                <span className={styles.choiceValue}>{option.value}</span>
              )}
              {option.value === initial && <Icon name="check" size={13} />}
            </li>
          ))}
        </ul>
      )}
    </span>
  )
}
