import { useState, type FormEvent } from 'react'
import { Button } from '../../components/Button'
import { Modal } from '../../components/Modal'
import { TextField } from '../../components/TextField'
import type { Connection, NewObjectKind } from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { adminApi } from './api'
import {
  EMPTY_VALUES,
  fieldsOf,
  isComplete,
  createLabelOf,
  noteOf,
  titleOf,
  PLACEHOLDERS,
  toNewObject,
  type ExtraField,
  type FieldValues,
} from './newObject'
import { messages, type AdminMessages } from './messages'
import styles from './NewObjectDialog.module.css'

function labelOf(field: ExtraField, m: AdminMessages): string {
  return { password: m.password, host: m.host, collection: m.firstCollection, database: m.database }[field]
}

interface NewObjectDialogProps {
  connection: Connection
  kind: NewObjectKind
  onClose: () => void
  /** The object exists on the server; a new database then shows in the explorer. */
  onCreated: () => void
}

/** Creates a database, role or user on a connection's server. */
export function NewObjectDialog({ connection, kind, onClose, onCreated }: NewObjectDialogProps) {
  const fields = fieldsOf(connection.engine, kind)
  const [values, setValues] = useState<FieldValues>(EMPTY_VALUES)
  const [error, setError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const m = useMessages(messages)

  const change = (field: keyof FieldValues, value: string) => {
    setValues({ ...values, [field]: value })
    setError(null)
  }

  const create = async (event: FormEvent) => {
    event.preventDefault()
    if (creating || !isComplete(values, fields)) return
    setCreating(true)
    try {
      await adminApi.create(connection.id, toNewObject(kind, fields, values))
      onCreated()
      onClose()
    } catch (failure) {
      setError(errorMessage(failure))
      setCreating(false)
    }
  }

  return (
    <Modal title={titleOf(kind, connection.name, m)} onClose={onClose} className={styles.dialog}>
      <form onSubmit={(event) => void create(event)}>
        <div className={styles.body}>
          <TextField
            label={m.name}
            mono
            data-autofocus
            value={values.name}
            disabled={creating}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => change('name', event.target.value)}
          />
          {fields.map((field) => (
            <TextField
              key={field}
              label={labelOf(field, m)}
              mono={field !== 'password'}
              type={field === 'password' ? 'password' : 'text'}
              placeholder={PLACEHOLDERS[field]}
              value={values[field]}
              disabled={creating}
              spellCheck={false}
              autoComplete={field === 'password' ? 'new-password' : 'off'}
              onChange={(event) => change(field, event.target.value)}
            />
          ))}
          <p className={styles.note}>{noteOf(connection.engine, kind, m)}</p>
          {/* Always there, so an error never moves the buttons. */}
          <p className={styles.error} role="alert">
            {error}
          </p>
        </div>
        <footer className={styles.footer}>
          <Button type="button" onClick={onClose}>
            {m.cancel}
          </Button>
          <Button type="submit" variant="primary" disabled={creating || !isComplete(values, fields)}>
            {creating ? m.creating : createLabelOf(kind, m)}
          </Button>
        </footer>
      </form>
    </Modal>
  )
}
