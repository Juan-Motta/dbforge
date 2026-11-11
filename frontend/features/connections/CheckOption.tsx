import type { ReactNode } from 'react'
import styles from './ConnectionDialog.module.css'

/** A checkbox with what it does under its label. */
export function CheckOption({
  label,
  help,
  checked,
  disabled,
  onChange,
}: {
  label: string
  help: ReactNode
  checked: boolean
  disabled?: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <label className={styles.check}>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>
        {label}
        <span className={styles.optionHelp}>{help}</span>
      </span>
    </label>
  )
}
