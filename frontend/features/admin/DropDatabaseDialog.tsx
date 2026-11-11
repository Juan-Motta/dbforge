import { useState, type FormEvent } from 'react'
import { Button } from '../../components/Button'
import { Modal } from '../../components/Modal'
import { TextField } from '../../components/TextField'
import type { Connection } from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { adminApi } from './api'
import { messages } from './messages'
import styles from './DropDatabaseDialog.module.css'

interface DropDatabaseDialogProps {
  connection: Connection
  database: string
  onClose: () => void
  /** The database is gone from the server. */
  onDropped: () => void
}

/**
 * Drops a database from its server once its name is typed again, so a
 * misplaced click or a database picked by mistake can't lose it.
 */
export function DropDatabaseDialog({ connection, database, onClose, onDropped }: DropDatabaseDialogProps) {
  const m = useMessages(messages)
  const [typed, setTyped] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [dropping, setDropping] = useState(false)
  const confirmed = typed === database

  const drop = async (event: FormEvent) => {
    event.preventDefault()
    if (dropping || !confirmed) return
    setDropping(true)
    try {
      await adminApi.dropDatabase(connection.id, database)
      onDropped()
      onClose()
    } catch (failure) {
      setError(errorMessage(failure))
      setDropping(false)
    }
  }

  return (
    <Modal title={m.dropTitle(database)} onClose={onClose} className={styles.dialog}>
      <form onSubmit={(event) => void drop(event)}>
        <div className={styles.body}>
          <p className={styles.note}>{m.dropNote(database, connection.name)}</p>
          <TextField
            label={m.dropConfirm(database)}
            mono
            data-autofocus
            value={typed}
            disabled={dropping}
            autoComplete="off"
            onChange={(event) => {
              setTyped(event.target.value)
              setError(null)
            }}
          />
          {/* Always there, so an error never moves the buttons. */}
          <p className={styles.error} role="alert">
            {error}
          </p>
        </div>
        <footer className={styles.footer}>
          <Button type="button" onClick={onClose}>
            {m.cancel}
          </Button>
          <Button type="submit" variant="danger" disabled={dropping || !confirmed}>
            {dropping ? m.dropping : m.dropDatabase}
          </Button>
        </footer>
      </form>
    </Modal>
  )
}
