import { useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { Badge } from '../../components/Badge'
import { Button } from '../../components/Button'
import { EngineLogo } from '../../components/EngineLogo'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { TextField } from '../../components/TextField'
import {
  ENGINE_LABELS,
  ENVIRONMENT_LABELS,
  isFileEngine,
  ACCOUNT_ENGINES,
  NOSQL_ENGINES,
  RELATIONAL_ENGINES,
  TLS_ENGINES,
  type Connection,
  type Engine,
  type Environment,
} from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import { connectionsApi, type ConnectionInput } from './api'
import {
  canSubmit,
  canTest,
  emptyForm,
  formFromConnection,
  isValidPort,
  parseConnectionUrl,
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

/** Engines whose host field also takes a pasted URL, which it splits into the fields. */
const HOST_HINTS: Partial<Record<Engine, string>> = {
  mongodb: 'localhost, or paste a mongodb:// URL',
  redis: 'localhost, or paste a redis:// URL',
}

export function ConnectionDialog({ connection, onSave, onDelete, onClose }: ConnectionDialogProps) {
  const [form, setForm] = useState<ConnectionForm>(() =>
    connection ? formFromConnection(connection) : emptyForm(),
  )
  const account = ACCOUNT_ENGINES.includes(form.engine)
  const [test, setTest] = useState<TestState>({ status: 'idle' })
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [confirmingDelete, setConfirmingDelete] = useState(false)
  /** What a pasted URL held that the form can't, or `null` when nothing was pasted. */
  const [fromUrl, setFromUrl] = useState<string[] | null>(null)

  const update = (changes: Partial<ConnectionForm>) => {
    setForm((current) => ({ ...current, ...changes }))
    setTest({ status: 'idle' })
  }

  /** Splits a connection URL into the fields; false when `text` isn't one. */
  const fillFromUrl = (text: string) => {
    const parsed = parseConnectionUrl(text)
    if (!parsed) return false
    setForm((current) => ({ ...withEngine(current, parsed.changes.engine!), ...parsed.changes }))
    setTest({ status: 'idle' })
    setFromUrl(parsed.ignored)
    return true
  }

  /** Picks the SQLite file with the native dialog; cancelling keeps the current path. */
  const browse = async () => {
    const path = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: 'SQLite database', extensions: ['db', 'sqlite', 'sqlite3', 'db3'] },
        { name: 'All files', extensions: ['*'] },
      ],
    })
    if (typeof path === 'string') update({ database: path })
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

          {isFileEngine(form.engine) ? (
            <>
              <div className={styles.fileRow}>
                <TextField
                  label="Database file"
                  mono
                  placeholder="/path/to/database.db"
                  value={form.database}
                  onChange={(e) => update({ database: e.target.value })}
                />
                <Button onClick={browse}>Browse…</Button>
              </div>
              <p className={styles.hint}>The file must exist; DBForge never creates one by mistake.</p>
            </>
          ) : (
            <>
              {account ? (
                <TextField
                  label="URI"
                  mono
                  placeholder="https://<account>.documents.azure.com:443/, or paste a connection string"
                  aria-invalid={form.host.trim() !== '' && !canTest(form)}
                  value={form.host}
                  onChange={(e) => update({ host: e.target.value })}
                  // A connection string fills the URI and the key.
                  onPaste={(e) => {
                    if (fillFromUrl(e.clipboardData.getData('text'))) e.preventDefault()
                  }}
                  onBlur={() => fillFromUrl(form.host)}
                />
              ) : (
                <div className={styles.hostRow}>
                  <TextField
                    label="Host"
                    mono
                    placeholder={HOST_HINTS[form.engine]}
                    value={form.host}
                    onChange={(e) => update({ host: e.target.value })}
                    // A connection URL, pasted or typed, fills the fields instead of staying in Host.
                    onPaste={(e) => {
                      if (fillFromUrl(e.clipboardData.getData('text'))) e.preventDefault()
                    }}
                    onBlur={() => fillFromUrl(form.host)}
                  />
                  <TextField
                    label="Port"
                    mono
                    inputMode="numeric"
                    disabled={form.engine === 'mongodb' && form.srv}
                    aria-invalid={!(form.engine === 'mongodb' && form.srv) && !isValidPort(form.port)}
                    value={form.engine === 'mongodb' && form.srv ? '' : form.port}
                    placeholder={form.engine === 'mongodb' && form.srv ? 'From DNS' : undefined}
                    onChange={(e) => update({ port: e.target.value })}
                  />
                </div>
              )}
              {fromUrl && (
                <p className={styles.hint} role="status">
                  Filled in from the {account ? 'connection string' : 'URL'}.
                  {fromUrl.length > 0 && (
                    <span className={styles.ignored}> Not carried over: {fromUrl.join(', ')}.</span>
                  )}
                </p>
              )}

              {form.engine !== 'redis' && !account && (
                <TextField
                  label="Database"
                  mono
                  value={form.database}
                  onChange={(e) => update({ database: e.target.value })}
                />
              )}

              <div className={account ? undefined : styles.pair}>
                {!account && (
                  <TextField
                    // Redis 6+ has ACL users; without ACLs the password alone is enough.
                    label={form.engine === 'redis' ? 'User (ACL, optional)' : 'User'}
                    placeholder={form.engine === 'redis' ? 'default' : undefined}
                    mono
                    autoComplete="off"
                    value={form.username}
                    onChange={(e) => update({ username: e.target.value })}
                  />
                )}
                <TextField
                  label={account ? 'Key' : 'Password'}
                  mono
                  type="password"
                  autoComplete="new-password"
                  placeholder={connection ? 'Unchanged' : account ? 'Primary or secondary key' : ''}
                  value={form.password}
                  onChange={(e) => update({ password: e.target.value })}
                />
              </div>
              <p className={styles.hint}>
                {account ? 'Keys' : 'Passwords'} are stored in your OS keychain, never in a file.
              </p>

              {TLS_ENGINES.includes(form.engine) && (
                <div className={styles.options}>
                  {form.engine === 'mongodb' && (
                    <label className={styles.check}>
                      <input
                        type="checkbox"
                        checked={form.srv}
                        onChange={(e) => update({ srv: e.target.checked, tls: form.tls || e.target.checked })}
                      />
                      DNS seed list (mongodb+srv), as Atlas uses
                    </label>
                  )}
                  {!account && (
                    <label className={styles.check}>
                      <input
                        type="checkbox"
                        checked={form.tls || (form.engine === 'mongodb' && form.srv)}
                        disabled={form.engine === 'mongodb' && form.srv}
                        onChange={(e) => update({ tls: e.target.checked })}
                      />
                      Use TLS
                    </label>
                  )}
                  <label className={styles.check}>
                    <input
                      type="checkbox"
                      checked={form.allowSelfSigned}
                      disabled={
                        account
                          ? !/^https:/i.test(form.host.trim())
                          : !(form.tls || (form.engine === 'mongodb' && form.srv))
                      }
                      onChange={(e) => update({ allowSelfSigned: e.target.checked })}
                    />
                    Accept self-signed certificate
                  </label>
                </div>
              )}
            </>
          )}
        </form>
      </div>

      <footer className={styles.footer}>
        <Button onClick={runTest} disabled={test.status === 'testing' || !canTest(form)}>
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
