import { useState } from 'react'
import { Button } from '../../components/Button'
import { Modal } from '../../components/Modal'
import type { Connection } from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import styles from './DeleteConnectionDialog.module.css'

interface DeleteConnectionDialogProps {
  connection: Connection
  onDelete: (id: string) => Promise<void>
  onClose: () => void
}

/** Asks before removing a saved connection; its databases on the server are untouched. */
export function DeleteConnectionDialog({ connection, onDelete, onClose }: DeleteConnectionDialogProps) {
  const [error, setError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState(false)

  const remove = async () => {
    setDeleting(true)
    try {
      await onDelete(connection.id)
      onClose()
    } catch (failure) {
      setError(errorMessage(failure))
      setDeleting(false)
    }
  }

  return (
    <Modal title={`Delete ${connection.name}?`} onClose={onClose} className={styles.dialog}>
      <div className={styles.body}>
        <p>
          DBForge forgets this connection and its saved password, and closes its tabs. Nothing on the server
          changes.
        </p>
        <p className={styles.error} role="alert">
          {error}
        </p>
      </div>
      <footer className={styles.footer}>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" data-autofocus disabled={deleting} onClick={() => void remove()}>
          {deleting ? 'Deleting…' : 'Delete connection'}
        </Button>
      </footer>
    </Modal>
  )
}
