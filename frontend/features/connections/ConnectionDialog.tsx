import { useEffect, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { Badge } from '../../components/Badge'
import { Button, IconButton } from '../../components/Button'
import { EngineLogo } from '../../components/EngineLogo'
import { Icon } from '../../components/Icon'
import { InfoTip } from '../../components/InfoTip'
import { Modal } from '../../components/Modal'
import { Select, type SelectOption } from '../../components/Select'
import { TextField } from '../../components/TextField'
import { ENGINE_LIST, ENGINES, isFileEngine, type Connection, type Engine } from '../../domain/connection'
import { copyText } from '../../lib/clipboard'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { tagColor } from '../tags/tags'
import { useTags } from '../tags/useTags'
import { connectionsApi, type ConnectionInput, type CredentialStorage } from './api'
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
import { connectionUrl } from './url'
import styles from './ConnectionDialog.module.css'

type TestState = { status: 'idle' } | { status: 'testing' } | { status: 'ok' | 'error'; message: string }

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
  const { tags } = useTags()
  const [storage, setStorage] = useState<CredentialStorage | null>(null)

  useEffect(() => {
    let current = true
    connectionsApi
      .credentialStorage()
      .then((info) => current && setStorage(info.storage))
      .catch(() => {})
    return () => {
      current = false
    }
  }, [])
  const [form, setForm] = useState<ConnectionForm>(() =>
    connection ? formFromConnection(connection) : emptyForm(),
  )
  const account = ENGINES[form.engine].reach === 'account'
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

  const engineOptions: SelectOption<Engine>[] = [
    ...ENGINE_LIST.map((engine) => ({
      engine,
      group: ENGINES[engine].family === 'relational' ? m.relational : m.nosql,
    })),
  ].map(({ engine, group }) => ({
    value: engine,
    label: ENGINES[engine].label,
    group,
    icon: <EngineLogo engine={engine} size={16} />,
  }))
  // '' stands for no tag: a dropdown always has a value.
  const tagOptions: SelectOption<string>[] = [
    { value: '', label: m.noTag, icon: <span className={styles.dot} /> },
    ...tags.map((tag) => ({
      value: tag.id,
      label: tag.name,
      icon: <span className={styles.dot} style={{ background: tagColor(tag.color) }} />,
    })),
  ]
  const srv = form.engine === 'mongodb' && form.srv
  const database = databaseField(form.engine, m)

  return (
    <Modal
      title={connection ? m.editConnection : m.newConnection}
      onClose={onClose}
      className={styles.dialog}
    >
      <form
        id="connection-form"
        className={styles.form}
        onSubmit={(event) => {
          event.preventDefault()
          if (canSubmit(form)) void save()
        }}
      >
        <div className={styles.pair}>
          <Select
            label={m.engine}
            info={m.engineInfo}
            value={form.engine}
            options={engineOptions}
            onChange={(engine) => {
              setForm((current) => withEngine(current, engine))
              setTest({ status: 'idle' })
              setFromUrl(null)
            }}
          />
          <Select
            label={m.environment}
            info={m.tagInfo}
            value={form.environment ?? ''}
            options={tagOptions}
            onChange={(id) => update({ environment: id || null })}
          />
        </div>

        <TextField
          label={m.name}
          help={m.nameHelp}
          data-autofocus
          required
          value={form.name}
          onChange={(e) => update({ name: e.target.value })}
        />

        {isFileEngine(form.engine) ? (
          <div className={styles.fileRow}>
            <TextField
              label={m.databaseFile}
              info={m.fileInfo}
              help={m.fileHint}
              mono
              placeholder={m.databaseFilePlaceholder}
              value={form.database}
              onChange={(e) => update({ database: e.target.value })}
            />
            <Button onClick={browse}>{m.browse}</Button>
          </div>
        ) : (
          <>
            {account ? (
              <TextField
                label={m.accountUri}
                info={m.uriInfo}
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
                  info={hostInfoOf(form.engine, m)}
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
                  info={m.portInfo(ENGINES[form.engine].defaultPort)}
                  mono
                  inputMode="numeric"
                  disabled={srv}
                  aria-invalid={!srv && !isValidPort(form.port)}
                  value={srv ? '' : form.port}
                  placeholder={srv ? m.portFromDns : undefined}
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

            {database && (
              <TextField
                label={database.label}
                help={database.help}
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
                  info={userInfoOf(form.engine, m)}
                  placeholder={form.engine === 'redis' ? 'default' : undefined}
                  mono
                  autoComplete="off"
                  value={form.username}
                  onChange={(e) => update({ username: e.target.value })}
                />
              )}
              <TextField
                label={account ? m.key : m.password}
                info={account ? m.keyInfo : m.passwordInfo}
                mono
                type="password"
                autoComplete="new-password"
                placeholder={connection ? m.unchanged : account ? m.keyPlaceholder : ''}
                value={form.password}
                onChange={(e) => update({ password: e.target.value })}
              />
            </div>
            {/* The line is kept while the storage loads, so the form never moves. */}
            <p className={styles.hint}>{storageNote(storage, account, m)}</p>

            {ENGINES[form.engine].tls && (
              <fieldset className={styles.options}>
                <legend className={styles.legend}>{m.security}</legend>
                {form.engine === 'mongodb' && (
                  <Option
                    label={m.srv}
                    help={m.srvHelp}
                    checked={form.srv}
                    onChange={(checked) => update({ srv: checked, tls: form.tls || checked })}
                  />
                )}
                {!account && (
                  <Option
                    label={m.useTls}
                    help={m.tlsHelp}
                    checked={form.tls || srv}
                    disabled={srv}
                    onChange={(checked) => update({ tls: checked })}
                  />
                )}
                <Option
                  label={m.selfSigned}
                  help={m.selfSignedHelp}
                  checked={form.allowSelfSigned}
                  disabled={account ? !/^https:/i.test(form.host.trim()) : !(form.tls || srv)}
                  onChange={(checked) => update({ allowSelfSigned: checked })}
                />
              </fieldset>
            )}

            <UrlPreview form={form} m={m} />
          </>
        )}

        <Select
          label={m.agentAccess}
          info={m.agentAccessInfo}
          value={form.agentAccess}
          options={[
            { value: 'none', label: m.agentNone },
            { value: 'read', label: m.agentRead },
            { value: 'write', label: m.agentWrite },
          ]}
          onChange={(agentAccess) => update({ agentAccess })}
        />
      </form>

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

/** The database field in each engine's terms; none for Redis, whose numbered databases are all listed. */
function databaseField(engine: Engine, m: ConnectionMessages): { label: string; help: string } | null {
  switch (engine) {
    case 'redis':
    case 'cosmosdb':
    case 'sqlite':
      return null
    case 'postgres':
      return { label: m.database, help: m.databaseHelp }
    case 'sqlserver':
      return { label: m.defaultDatabase, help: m.sqlServerDatabaseHelp }
    case 'mongodb':
      return { label: m.defaultDatabase, help: m.mongoDatabaseHelp }
    case 'mysql':
    case 'mariadb':
      return { label: m.defaultDatabase, help: m.optionalDatabaseHelp }
  }
}

function hostInfoOf(engine: Engine, m: ConnectionMessages): string {
  if (engine === 'mongodb') return m.hostPasteInfo('mongodb://')
  if (engine === 'redis') return m.hostPasteInfo('redis://')
  return m.hostInfo
}

function userInfoOf(engine: Engine, m: ConnectionMessages): string {
  if (engine === 'sqlserver') return m.sqlServerUserInfo
  if (engine === 'redis') return m.redisUserInfo
  return m.userInfo
}

/** A checkbox with what it does under its label. */
function Option({
  label,
  help,
  checked,
  disabled,
  onChange,
}: {
  label: string
  help: string
  checked: boolean
  disabled?: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <label className={styles.check}>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>
        {label}
        <span className={styles.optionHelp}>{help}</span>
      </span>
    </label>
  )
}

/** Where the fields point, without the password, ready to copy. */
function UrlPreview({ form, m }: { form: ConnectionForm; m: ConnectionMessages }) {
  const [copied, setCopied] = useState(false)
  // Until the fields could connect, there is nothing to point at; the row stays so the form never moves.
  const url = canTest(form) ? connectionUrl({ ...toInput(form), id: '' }) : null

  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 1500)
    return () => clearTimeout(timer)
  }, [copied])

  return (
    <div>
      <span className={styles.label}>
        {m.connectionUrl}
        <InfoTip text={m.connectionUrlInfo} />
      </span>
      <div className={styles.url}>
        <code title={url ?? undefined}>{url ?? '—'}</code>
        <IconButton
          label={copied ? m.copied : m.copyUrl}
          title={copied ? m.copied : m.copyUrl}
          disabled={url === null}
          onClick={() => url && void copyText(url).then(() => setCopied(true))}
        >
          <Icon name={copied ? 'check' : 'duplicate'} size={14} />
        </IconButton>
      </div>
    </div>
  )
}

/** Where the password (or account key) will be kept; a blank line while that loads. */
function storageNote(storage: CredentialStorage | null, account: boolean, m: ConnectionMessages): string {
  if (storage === null) return '\u00a0'
  if (storage === 'file') return account ? m.keysStoredFile : m.passwordsStoredFile
  return account ? m.keysStored : m.passwordsStored
}
