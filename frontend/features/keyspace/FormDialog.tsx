import { useState, type FormEvent, type ReactNode } from 'react'
import { Button } from '../../components/Button'
import { Modal } from '../../components/Modal'
import { errorMessage } from '../../lib/errors'
import styles from './KeyBrowser.module.css'

export interface FormField {
  name: string
  label: string
  type?: 'text' | 'textarea' | 'number' | 'select'
  options?: { value: string; label: string }[]
  placeholder?: string
  initial?: string
  /** Shown only while this holds for the current values. */
  when?: (values: Record<string, string>) => boolean
}

interface FormDialogProps {
  title: string
  submitLabel: string
  fields: FormField[]
  /** Text above the fields, e.g. what a deletion removes. */
  children?: ReactNode
  danger?: boolean
  /** Rejects with the backend's message, which the dialog shows. */
  onSubmit: (values: Record<string, string>) => Promise<void>
  onClose: () => void
  /** An extra action, such as "Remove TTL". */
  secondary?: { label: string; run: () => Promise<void> }
}

/** A small form in a modal; it stays open and shows the error when submitting fails. */
export function FormDialog({
  title,
  submitLabel,
  fields,
  children,
  danger,
  onSubmit,
  onClose,
  secondary,
}: FormDialogProps) {
  const [values, setValues] = useState(() => Object.fromEntries(fields.map((f) => [f.name, f.initial ?? ''])))
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const run = async (action: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await action()
      onClose()
    } catch (failure) {
      setError(errorMessage(failure))
      setBusy(false)
    }
  }

  const submit = (event: FormEvent) => {
    event.preventDefault()
    void run(() => onSubmit(values))
  }

  const shown = fields.filter((field) => !field.when || field.when(values))
  return (
    <Modal title={title} onClose={onClose} className={styles.dialog}>
      <form onSubmit={submit}>
        <div className={styles.dialogBody}>
          {children}
          {shown.map((field, index) => {
            const common = {
              id: `field-${field.name}`,
              value: values[field.name],
              placeholder: field.placeholder,
              'data-autofocus': index === 0 ? true : undefined,
              onChange: (event: { target: { value: string } }) =>
                setValues((current) => ({ ...current, [field.name]: event.target.value })),
            }
            return (
              <label key={field.name} className={styles.formField} htmlFor={common.id}>
                <span className={styles.fieldLabel}>{field.label}</span>
                {field.type === 'textarea' ? (
                  <textarea {...common} className={`${styles.input} ${styles.textarea}`} rows={5} />
                ) : field.type === 'select' ? (
                  <select {...common} className={styles.input}>
                    {field.options?.map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                ) : (
                  <input
                    {...common}
                    className={styles.input}
                    inputMode={field.type === 'number' ? 'decimal' : undefined}
                    spellCheck={false}
                  />
                )}
              </label>
            )
          })}
          {error && (
            <p className={styles.error} role="alert">
              {error}
            </p>
          )}
        </div>
        <footer className={styles.dialogFooter}>
          {secondary && (
            <Button type="button" disabled={busy} onClick={() => void run(secondary.run)}>
              {secondary.label}
            </Button>
          )}
          <span className={styles.spacer} />
          <Button type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant={danger ? 'default' : 'primary'}
            className={danger ? styles.danger : undefined}
            disabled={busy}
          >
            {submitLabel}
          </Button>
        </footer>
      </form>
    </Modal>
  )
}
