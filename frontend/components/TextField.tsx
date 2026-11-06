import type { InputHTMLAttributes } from 'react'
import styles from './TextField.module.css'

interface TextFieldProps extends InputHTMLAttributes<HTMLInputElement> {
  label: string
  /** Use the monospace font, for technical values like hosts. */
  mono?: boolean
}

export function TextField({ label, mono = false, className, ...rest }: TextFieldProps) {
  return (
    <label className={className}>
      <span className={styles.label}>{label}</span>
      <input className={`${styles.input} ${mono ? 'mono' : ''}`} spellCheck={false} {...rest} />
    </label>
  )
}
