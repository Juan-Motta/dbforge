import { useId, type InputHTMLAttributes, type ReactNode } from 'react'
import { InfoTip } from './InfoTip'
import styles from './TextField.module.css'

interface TextFieldProps extends InputHTMLAttributes<HTMLInputElement> {
  label: string
  /** Use the monospace font, for technical values like hosts. */
  mono?: boolean
  /** Explains the field in a tooltip next to its label. */
  info?: string
  /** A line under the field, e.g. what an empty value means; read with the field. */
  help?: ReactNode
}

export function TextField({ label, mono = false, info, help, className, ...rest }: TextFieldProps) {
  const helpId = useId()
  return (
    <div className={className}>
      <label>
        <span className={styles.label}>
          {label}
          {info && <InfoTip text={info} />}
        </span>
        <input
          className={`${styles.input} ${mono ? 'mono' : ''}`}
          spellCheck={false}
          aria-describedby={help ? helpId : undefined}
          {...rest}
        />
      </label>
      {help && (
        <p id={helpId} className={styles.help}>
          {help}
        </p>
      )}
    </div>
  )
}
