import { useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { Icon } from './Icon'
import { InfoTip } from './InfoTip'
import { matchTyped, step } from './selectKeys'
import styles from './Select.module.css'

export interface SelectOption<T extends string> {
  value: T
  label: string
  /** Drawn before the label, e.g. an engine's logo or a tag's color. */
  icon?: ReactNode
  /** Options sharing a group are listed under its heading. */
  group?: string
}

interface SelectProps<T extends string> {
  label: string
  value: T
  options: SelectOption<T>[]
  onChange: (value: T) => void
  /** Explains the field in a tooltip next to its label. */
  info?: string
  autoFocus?: boolean
  /** The label is only read by screen readers, for a field whose row already says what it is. */
  hideLabel?: boolean
}

/** Where the list opens, in viewport pixels: below the button, or above it when there is no room. */
interface Placement {
  left: number
  width: number
  top?: number
  bottom?: number
}

const LIST_MAX_HEIGHT = 320

/** A dropdown: a button showing the choice, and a list to pick from with the mouse or the keyboard. */
export function Select<T extends string>({
  label,
  value,
  options,
  onChange,
  info,
  autoFocus,
  hideLabel = false,
}: SelectProps<T>) {
  const id = useId()
  const button = useRef<HTMLButtonElement>(null)
  const [placement, setPlacement] = useState<Placement | null>(null)
  const [active, setActive] = useState(-1)
  // Letters typed in a row jump to the option they start; the run ends after a pause.
  const typed = useRef({ text: '', at: 0 })
  const selected = options.find((option) => option.value === value)
  const selectedIndex = options.findIndex((option) => option.value === value)

  const open = () => {
    const rect = button.current!.getBoundingClientRect()
    const below = window.innerHeight - rect.bottom
    setPlacement(
      below >= Math.min(LIST_MAX_HEIGHT, 40 * options.length) || below >= rect.top
        ? { left: rect.left, width: rect.width, top: rect.bottom + 4 }
        : { left: rect.left, width: rect.width, bottom: window.innerHeight - rect.top + 4 },
    )
    setActive(selectedIndex)
  }

  const close = () => {
    setPlacement(null)
    button.current?.focus()
  }

  const choose = (index: number) => {
    const option = options[index]
    if (option) onChange(option.value)
    close()
  }

  const onKeyDown = (event: KeyboardEvent) => {
    const keys: Record<string, () => void> = placement
      ? {
          ArrowDown: () => setActive((current) => step(current, 1, options.length)),
          ArrowUp: () => setActive((current) => step(current, -1, options.length)),
          Home: () => setActive(0),
          End: () => setActive(options.length - 1),
          Enter: () => choose(active),
          ' ': () => choose(active),
          // Inside a dialog, Escape must close the list, not the dialog.
          Escape: close,
          Tab: () => setPlacement(null),
        }
      : { ArrowDown: open, ArrowUp: open, Enter: open, ' ': open }
    const run = keys[event.key]
    if (run) {
      if (event.key !== 'Tab') event.preventDefault()
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
      const labels = options.map((option) => option.label)
      const from =
        typed.current.text.length > 1 ? Math.max(active, 0) : (Math.max(active, 0) + 1) % options.length
      const match = matchTyped(labels, typed.current.text, from)
      if (match < 0) return
      if (placement) setActive(match)
      else onChange(options[match].value)
    }
  }

  return (
    <div className={styles.field}>
      <span className={hideLabel ? styles.hiddenLabel : styles.label} id={`${id}-label`}>
        {label}
        {info && <InfoTip text={info} />}
      </span>
      <button
        ref={button}
        type="button"
        role="combobox"
        className={styles.button}
        aria-haspopup="listbox"
        aria-expanded={placement !== null}
        aria-controls={placement ? `${id}-list` : undefined}
        aria-activedescendant={placement && active >= 0 ? `${id}-${active}` : undefined}
        aria-labelledby={`${id}-label ${id}-value`}
        data-autofocus={autoFocus || undefined}
        onClick={() => (placement ? close() : open())}
        onKeyDown={onKeyDown}
      >
        {selected?.icon}
        <span className={styles.value} id={`${id}-value`}>
          {selected?.label}
        </span>
        <Icon name="chevronDown" size={12} />
      </button>
      {placement && (
        // Fixed to the window: the dialog around it clips whatever overflows.
        <div className={styles.backdrop} onPointerDown={() => setPlacement(null)}>
          <ul
            id={`${id}-list`}
            role="listbox"
            aria-labelledby={`${id}-label`}
            className={styles.list}
            style={{ ...placement, maxHeight: LIST_MAX_HEIGHT }}
            onPointerDown={(event) => event.stopPropagation()}
          >
            {options.map((option, index) => (
              <Option
                key={option.value}
                id={`${id}-${index}`}
                option={option}
                heading={option.group !== undefined && option.group !== options[index - 1]?.group}
                selected={option.value === value}
                active={index === active}
                onHover={() => setActive(index)}
                onPick={() => choose(index)}
              />
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

interface OptionProps<T extends string> {
  id: string
  option: SelectOption<T>
  heading: boolean
  selected: boolean
  active: boolean
  onHover: () => void
  onPick: () => void
}

function Option<T extends string>({
  id,
  option,
  heading,
  selected,
  active,
  onHover,
  onPick,
}: OptionProps<T>) {
  return (
    <>
      {heading && (
        <li role="presentation" className={styles.group}>
          {option.group}
        </li>
      )}
      <li
        id={id}
        role="option"
        aria-selected={selected}
        className={active ? `${styles.option} ${styles.active}` : styles.option}
        // Keeps the highlighted option in view while the keyboard moves it.
        ref={(element) => {
          if (active) element?.scrollIntoView({ block: 'nearest' })
        }}
        onPointerMove={onHover}
        onClick={onPick}
      >
        {option.icon}
        <span className={styles.value}>{option.label}</span>
        {selected && <Icon name="check" size={13} />}
      </li>
    </>
  )
}
