import { useState } from 'react'
import { Badge } from '../../components/Badge'
import { Button } from '../../components/Button'
import { EngineLogo } from '../../components/EngineLogo'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { TextField } from '../../components/TextField'
import {
  ENGINE_LABELS,
  ENVIRONMENT_LABELS,
  NOSQL_ENGINES,
  RELATIONAL_ENGINES,
  type Connection,
  type Engine,
  type Environment,
} from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import { connectionsApi, type ConnectionInput } from './api'
import {
  canSubmit,
  emptyForm,
  formFromConnection,
  isValidPort,
  toInput,
  withEngine,
  type ConnectionForm,
} from './form'
import styles from './ConnectionDialog.module.css'

type TestState = { status: 'idle' } | { status: 'testing' } | { status: 'ok' | 'error'; message: string }

const ENVIRONMENT_TONES: Record<Environment, string> = {
  development: 'var(--info)',
  staging: 'var(--warn)',
  production: 'var(--err)',
}

interface ConnectionDialogProps {
  /** The connection to edit; omit to create a new one. */
  connection?: Connection
  onSave: (input: ConnectionInput) => Promise<unknown>
  onDelete: (id: string) => Promise<void>
  onClose: () => void
}

export function ConnectionDialog({ connection, onSave, onDelete, onClose }: ConnectionDialogProps) {
  const [form, setForm] = useState<ConnectionForm>(() =>
    connection ? formFromConnection(connection) : emptyForm(),
  )
  const [test, setTest] = useState<TestState>({ status: 'idle' })
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [confirmingDelete, setConfirmingDelete] = useState(false)

  const update = (changes: Partial<ConnectionForm>) => {
    setForm((current) => ({ ...current, ...changes }))
    setTest({ status: 'idle' })
  }

  const runTest = async () => {
    setTest({ status: 'testing' })
    try {
      const version = await connectionsApi.test(toInput(form, connection?.id))
      setTest({ status: 'ok', message: `Connected · ${version}` })
    } catch (error) {
      setTest({ status: 'error', message: errorMessage(error) })
    }
  }

  const save = async () => {
    setSaving(true)
    setSaveError(null)
    try {
      await onSave(toInput(form, connection?.id))
      onClose()
    } catch (error) {
      setSaveError(errorMessage(error))
      setSaving(false)
    }
  }

  const remove = async () => {
    if (!connection) return
    try {
      await onDelete(connection.id)
      onClose()
    } catch (error) {
      setSaveError(errorMessage(error))
    }
  }

  const engineButton = (engine: Engine) => (
    <button
      key={engine}
      type="button"
      className={styles.engine}
      aria-pressed={form.engine === engine}
      onClick={() => {
        setForm((current) => withEngine(current, engine))
        setTest({ status: 'idle' })
      }}
    >
      <EngineLogo engine={engine} size={20} />
      {ENGINE_LABELS[engine]}
    </button>
  )

  return (
    <Modal
      title={connection ? 'Edit connection' : 'New connection'}
      onClose={onClose}
      className={styles.dialog}
    >
      <div className={styles.body}>
        <nav className={styles.engines} aria-label="Database engine">
          <span className={styles.group}>Relational</span>
          {RELATIONAL_ENGINES.map(engineButton)}
          <span className={styles.group}>NoSQL</span>
          {NOSQL_ENGINES.map(engineButton)}
        </nav>

        <form
          id="connection-form"
          className={styles.form}
          onSubmit={(event) => {
            event.preventDefault()
            if (canSubmit(form)) void save()
          }}
        >
          <div className={styles.row}>
            <TextField
              label="Name"
              data-autofocus
              required
              value={form.name}
              onChange={(e) => update({ name: e.target.value })}
            />
            <div role="group" aria-labelledby="environment-label">
              <span id="environment-label" className={styles.label}>
                Environment
              </span>
              <div className={styles.chips}>
                {(Object.keys(ENVIRONMENT_LABELS) as Environment[]).map((env) => (
                  <button
                    key={env}
                    type="button"
                    className={styles.chip}
                    aria-pressed={form.environment === env}
                    onClick={() => update({ environment: form.environment === env ? null : env })}
                  >
                    <span className={styles.dot} style={{ background: ENVIRONMENT_TONES[env] }} />
                    {ENVIRONMENT_LABELS[env]}
                  </button>
                ))}
              </div>
            </div>
          </div>

          <div className={styles.hostRow}>
            <TextField
              label="Host"
              mono
              value={form.host}
              onChange={(e) => update({ host: e.target.value })}
            />
            <TextField
              label="Port"
              mono
              inputMode="numeric"
              aria-invalid={!isValidPort(form.port)}
              value={form.port}
              onChange={(e) => update({ port: e.target.value })}
            />
          </div>

          <TextField
            label="Database"
            mono
            value={form.database}
            onChange={(e) => update({ database: e.target.value })}
          />

          <div className={styles.pair}>
            <TextField
              label="User"
              mono
              autoComplete="off"
              value={form.username}
              onChange={(e) => update({ username: e.target.value })}
            />
            <TextField
              label="Password"
              mono
              type="password"
              autoComplete="new-password"
              placeholder={connection ? 'Unchanged' : ''}
              value={form.password}
              onChange={(e) => update({ password: e.target.value })}
            />
          </div>
          <p className={styles.hint}>Passwords are stored in your OS keychain, never in a file.</p>
        </form>
      </div>

      <footer className={styles.footer}>
        <Button onClick={runTest} disabled={test.status === 'testing' || !isValidPort(form.port)}>
          <Icon name="pulse" />
          {test.status === 'testing' ? 'Testing…' : 'Test connection'}
        </Button>
        <span role="status" className={styles.result}>
          {test.status === 'ok' && <Badge tone="ok">{test.message}</Badge>}
          {test.status === 'error' && <span className={styles.error}>{test.message}</span>}
          {saveError && <span className={styles.error}>{saveError}</span>}
        </span>

        {connection &&
          (confirmingDelete ? (
            <Button className={styles.danger} onClick={remove}>
              Confirm delete
            </Button>
          ) : (
            <Button variant="ghost" onClick={() => setConfirmingDelete(true)}>
              Delete
            </Button>
          ))}
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" form="connection-form" variant="primary" disabled={!canSubmit(form) || saving}>
          {saving ? 'Saving…' : 'Save'}
        </Button>
      </footer>
    </Modal>
  )
}
