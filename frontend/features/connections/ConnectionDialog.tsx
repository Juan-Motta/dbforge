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
  ENVIRONMENTS,
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
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages as sharedMessages } from '../../lib/messages'
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
import { messages, type ConnectionMessages } from './messages'
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
  onClose: () => void
}

/** Engines whose host field also takes a pasted URL, which it splits into the fields. */
function hostHintOf(engine: Engine, m: ConnectionMessages): string | undefined {
  if (engine === 'mongodb') return m.mongoHostHint
  if (engine === 'redis') return m.redisHostHint
  return undefined
}

export function ConnectionDialog({ connection, onSave, onClose }: ConnectionDialogProps) {
  const m = useMessages(messages)
  const shared = useMessages(sharedMessages)
  const [form, setForm] = useState<ConnectionForm>(() =>
    connection ? formFromConnection(connection) : emptyForm(),
  )
  const account = ACCOUNT_ENGINES.includes(form.engine)
  const [test, setTest] = useState<TestState>({ status: 'idle' })
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  /** What a pasted URL held that the form can't, or `null` when nothing was pasted. */
  const [fromUrl, setFromUrl] = useState<string[] | null>(null)

  const update = (changes: Partial<ConnectionForm>) => {
    setForm((current) => ({ ...current, ...changes }))
    setTest({ status: 'idle' })
  }

  /** Splits a connection URL into the fields; false when `text` isn't one. */
  const fillFromUrl = (text: string) => {
    const parsed = parseConnectionUrl(text, m)
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
        { name: m.sqliteFiles, extensions: ['db', 'sqlite', 'sqlite3', 'db3'] },
        { name: m.allFiles, extensions: ['*'] },
      ],
    })
    if (typeof path === 'string') update({ database: path })
  }

  const runTest = async () => {
    setTest({ status: 'testing' })
    try {
      const version = await connectionsApi.test(toInput(form, connection?.id))
      setTest({ status: 'ok', message: m.connected(version) })
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
      title={connection ? m.editConnection : m.newConnection}
      onClose={onClose}
      className={styles.dialog}
    >
      <div className={styles.body}>
        <nav className={styles.engines} aria-label={m.engine}>
          <span className={styles.group}>{m.relational}</span>
          {RELATIONAL_ENGINES.map(engineButton)}
          <span className={styles.group}>{m.nosql}</span>
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
              label={m.name}
              data-autofocus
              required
              value={form.name}
              onChange={(e) => update({ name: e.target.value })}
            />
            <div role="group" aria-labelledby="environment-label">
              <span id="environment-label" className={styles.label}>
                {m.environment}
              </span>
              <div className={styles.chips}>
                {ENVIRONMENTS.map((env) => (
                  <button
                    key={env}
                    type="button"
                    className={styles.chip}
                    aria-pressed={form.environment === env}
                    onClick={() => update({ environment: form.environment === env ? null : env })}
                  >
                    <span className={styles.dot} style={{ background: ENVIRONMENT_TONES[env] }} />
                    {shared[env]}
                  </button>
                ))}
              </div>
            </div>
          </div>

          {isFileEngine(form.engine) ? (
            <>
              <div className={styles.fileRow}>
                <TextField
                  label={m.databaseFile}
                  mono
                  placeholder={m.databaseFilePlaceholder}
                  value={form.database}
                  onChange={(e) => update({ database: e.target.value })}
                />
                <Button onClick={browse}>{m.browse}</Button>
              </div>
              <p className={styles.hint}>{m.fileHint}</p>
            </>
          ) : (
            <>
              {account ? (
                <TextField
                  label={m.uri}
                  mono
                  placeholder={m.uriPlaceholder}
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
                    label={m.host}
                    mono
                    placeholder={hostHintOf(form.engine, m)}
                    value={form.host}
                    onChange={(e) => update({ host: e.target.value })}
                    // A connection URL, pasted or typed, fills the fields instead of staying in Host.
                    onPaste={(e) => {
                      if (fillFromUrl(e.clipboardData.getData('text'))) e.preventDefault()
                    }}
                    onBlur={() => fillFromUrl(form.host)}
                  />
                  <TextField
                    label={m.port}
                    mono
                    inputMode="numeric"
                    disabled={form.engine === 'mongodb' && form.srv}
                    aria-invalid={!(form.engine === 'mongodb' && form.srv) && !isValidPort(form.port)}
                    value={form.engine === 'mongodb' && form.srv ? '' : form.port}
                    placeholder={form.engine === 'mongodb' && form.srv ? m.portFromDns : undefined}
                    onChange={(e) => update({ port: e.target.value })}
                  />
                </div>
              )}
              {fromUrl && (
                <p className={styles.hint} role="status">
                  {account ? m.filledFromConnectionString : m.filledFromUrl}
                  {fromUrl.length > 0 && (
                    <span className={styles.ignored}>{m.notCarriedOver(fromUrl.join(', '))}</span>
                  )}
                </p>
              )}

              {form.engine !== 'redis' && !account && (
                <TextField
                  label={m.database}
                  mono
                  value={form.database}
                  onChange={(e) => update({ database: e.target.value })}
                />
              )}

              <div className={account ? undefined : styles.pair}>
                {!account && (
                  <TextField
                    // Redis 6+ has ACL users; without ACLs the password alone is enough.
                    label={form.engine === 'redis' ? m.redisUser : m.user}
                    placeholder={form.engine === 'redis' ? 'default' : undefined}
                    mono
                    autoComplete="off"
                    value={form.username}
                    onChange={(e) => update({ username: e.target.value })}
                  />
                )}
                <TextField
                  label={account ? m.key : m.password}
                  mono
                  type="password"
                  autoComplete="new-password"
                  placeholder={connection ? m.unchanged : account ? m.keyPlaceholder : ''}
                  value={form.password}
                  onChange={(e) => update({ password: e.target.value })}
                />
              </div>
              <p className={styles.hint}>{account ? m.keysStored : m.passwordsStored}</p>

              {TLS_ENGINES.includes(form.engine) && (
                <div className={styles.options}>
                  {form.engine === 'mongodb' && (
                    <label className={styles.check}>
                      <input
                        type="checkbox"
                        checked={form.srv}
                        onChange={(e) => update({ srv: e.target.checked, tls: form.tls || e.target.checked })}
                      />
                      {m.srv}
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
                      {m.useTls}
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
                    {m.selfSigned}
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
          {test.status === 'testing' ? m.testing : m.testConnection}
        </Button>
        <span role="status" className={styles.result}>
          {test.status === 'ok' && <Badge tone="ok">{test.message}</Badge>}
          {test.status === 'error' && <span className={styles.error}>{test.message}</span>}
          {saveError && <span className={styles.error}>{saveError}</span>}
        </span>

        <Button variant="ghost" onClick={onClose}>
          {m.cancel}
        </Button>
        <Button type="submit" form="connection-form" variant="primary" disabled={!canSubmit(form) || saving}>
          {saving ? m.saving : m.save}
        </Button>
      </footer>
    </Modal>
  )
}
