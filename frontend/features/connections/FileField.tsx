import type { ReactNode } from 'react'
import { open, type DialogFilter } from '@tauri-apps/plugin-dialog'
import { Button } from '../../components/Button'
import { TextField } from '../../components/TextField'
import styles from './ConnectionDialog.module.css'

interface FileFieldProps {
  label: string
  value: string
  onChange: (path: string) => void
  /**
   * The only files the native picker lets you choose, by extension. Leave it
   * out for files that may have none, such as SSH keys: macOS has no "all
   * files" choice, and takes `*` as a literal extension.
   */
  filters?: DialogFilter[]
  browseLabel: string
  info?: string
  help?: ReactNode
  placeholder?: string
  disabled?: boolean
  invalid?: boolean
}

/** A path on this computer, typed or picked with the native dialog; cancelling keeps the current path. */
export function FileField({
  label,
  value,
  onChange,
  filters,
  browseLabel,
  info,
  help,
  placeholder,
  disabled = false,
  invalid = false,
}: FileFieldProps) {
  const browse = async () => {
    const path = await open({ multiple: false, directory: false, filters })
    if (typeof path === 'string') onChange(path)
  }

  return (
    <div className={styles.fileRow}>
      <TextField
        label={label}
        info={info}
        help={help}
        mono
        placeholder={placeholder}
        disabled={disabled}
        aria-invalid={invalid}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      <Button onClick={browse} disabled={disabled}>
        {browseLabel}
      </Button>
    </div>
  )
}
