import { useEffect, useState } from 'react'
import { Badge } from '../../components/Badge'
import { Button, IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { errorMessage } from '../../lib/errors'
import { keyspaceApi, type Entry, type KeyEdit, type KeyValue } from './api'
import { FormDialog, type FormField } from './FormDialog'
import { asHex, asJson, formatBytes, formatTtl, kindLabel, parseFields, parseScore } from './keyspace'
import { kindClass } from './kindClass'
import styles from './KeyBrowser.module.css'

const PAGE = 200

/** How each type names its elements and columns. */
const SHAPES: Record<string, { noun: string; add?: string; key?: string; value: string; filter: boolean }> = {
  hash: { noun: 'fields', add: 'Add field', key: 'Field', value: 'Value', filter: true },
  list: { noun: 'items', add: 'Add item', key: 'Index', value: 'Value', filter: false },
  set: { noun: 'members', add: 'Add member', value: 'Member', filter: true },
  zset: { noun: 'members', add: 'Add member', value: 'Member', filter: true },
  stream: { noun: 'entries', add: 'Add entry', key: 'ID', value: 'Fields', filter: false },
  string: { noun: 'bytes', value: 'Value', filter: false },
}

type Dialog = 'ttl' | 'rename' | 'add' | 'delete' | null

interface KeyDetailProps {
  connectionId: string
  db: number
  name: string
  /** Changes when the CLI ran something, to read the key again. */
  reloadToken: number
  /** Keys were added, renamed or removed: the list scans again. */
  onKeysChanged: (selected: string | null) => void
}

/** One key: its type, TTL and size, its value a page at a time, and its edits. */
export function KeyDetail({ connectionId, db, name, reloadToken, onKeysChanged }: KeyDetailProps) {
  const [value, setValue] = useState<KeyValue | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [filter, setFilter] = useState('')
  const [loadingMore, setLoadingMore] = useState(false)
  const [picked, setPicked] = useState(0)
  const [dialog, setDialog] = useState<Dialog>(null)

  const load = async () => {
    try {
      const read = await keyspaceApi.readKey(connectionId, db, name, null, filter, PAGE)
      setValue(read)
      setError(null)
      setPicked((current) => Math.min(current, Math.max(read.entries.length - 1, 0)))
    } catch (failure) {
      setError(errorMessage(failure))
      setValue(null)
    }
  }

  useEffect(() => {
    void load()
    // `load` reads the values these name.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectionId, db, name, filter, reloadToken])

  if (error) return <p className={`${styles.detailEmpty} ${styles.error}`}>{error}</p>
  if (!value) return <p className={styles.detailEmpty}>Loading {name}…</p>

  const shape = SHAPES[value.kind]
  const edit = async (change: KeyEdit) => {
    await keyspaceApi.editKey(connectionId, db, name, value.kind, change)
    await load()
  }
  const loadMore = async () => {
    if (!value.next) return
    setLoadingMore(true)
    try {
      const more = await keyspaceApi.readKey(connectionId, db, name, value.next, filter, PAGE)
      setValue({ ...more, entries: [...value.entries, ...more.entries] })
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setLoadingMore(false)
    }
  }
  const entry = value.entries[picked] as Entry | undefined

  return (
    <section className={styles.detail} aria-label={`Key ${name}`}>
      <header className={styles.detailHeader}>
        <span className={`${styles.kind} ${styles.kindLarge} ${kindClass(value.kind)}`}>
          {kindLabel(value.kind)}
        </span>
        <span className={`${styles.mono} ${styles.keyName}`}>{value.name}</span>
        <IconButton label="Rename key" onClick={() => setDialog('rename')}>
          <Icon name="edit" />
        </IconButton>
        <span className={styles.spacer} />
        {value.ttlMs !== null && (
          <Badge tone="warn">
            <Icon name="history" size={12} />
            TTL {formatTtl(value.ttlMs, true)}
          </Badge>
        )}
        {shape && (
          <Badge>
            {value.length.toLocaleString('en-US')} {shape.noun}
          </Badge>
        )}
        {value.bytes !== null && <Badge>{formatBytes(value.bytes)}</Badge>}
        {value.encoding && <Badge>{value.encoding}</Badge>}
        <Button onClick={() => setDialog('ttl')}>Set TTL</Button>
        <Button className={styles.danger} onClick={() => setDialog('delete')}>
          Delete
        </Button>
      </header>

      {!shape ? (
        <p className={styles.detailEmpty}>
          {value.kind} keys come from a server module; use the CLI below to read them.
        </p>
      ) : value.kind === 'string' ? (
        <div className={styles.detailBody}>
          {entry && (
            <ValueViewer
              key={name}
              label={`Value · ${formatBytes(value.length)}${value.length > entry.size ? ' (first 1 MB)' : ''}`}
              entry={entry}
              editable={!entry.value.binary && value.length === entry.size}
              onSave={(text) => edit({ kind: 'setString', value: text })}
            />
          )}
        </div>
      ) : (
        <>
          <div className={styles.detailBar}>
            {shape.filter && (
              <input
                className={styles.input}
                style={{ width: 260 }}
                placeholder={`Filter ${shape.noun}`}
                aria-label={`Filter ${shape.noun}`}
                value={filter}
                onChange={(event) => setFilter(event.target.value)}
              />
            )}
            <span className={styles.spacer} />
            {shape.add && (
              <Button onClick={() => setDialog('add')}>
                <Icon name="plus" />
                {shape.add}
              </Button>
            )}
          </div>
          <div className={styles.detailBody}>
            <div className={styles.entries} role="table" aria-label={`${shape.noun} of ${name}`}>
              <div className={`${styles.entryRow} ${styles.entryHead}`} role="row">
                <span role="columnheader">#</span>
                {shape.key && <span role="columnheader">{shape.key}</span>}
                <span role="columnheader">{shape.value}</span>
                {value.kind === 'zset' && <span role="columnheader">Score</span>}
                <span role="columnheader" className={styles.right}>
                  Size
                </span>
              </div>
              {value.entries.map((item, index) => (
                <button
                  key={index}
                  type="button"
                  role="row"
                  aria-selected={index === picked}
                  className={styles.entryRow}
                  onClick={() => setPicked(index)}
                >
                  <span className={styles.muted}>{index + 1}</span>
                  {shape.key && (
                    <span className={`${styles.ellipsis} ${styles.field}`}>{item.key?.text}</span>
                  )}
                  <span className={styles.ellipsis}>{item.value.text}</span>
                  {value.kind === 'zset' && <span className={styles.number}>{item.score}</span>}
                  <span className={`${styles.muted} ${styles.right}`}>{formatBytes(item.size)}</span>
                </button>
              ))}
              {value.entries.length === 0 && <p className={styles.muted}>Nothing matches.</p>}
            </div>
            {value.next && (
              <Button
                variant="ghost"
                className={styles.more}
                disabled={loadingMore}
                onClick={() => void loadMore()}
              >
                {loadingMore ? 'Loading…' : `Load more (${value.entries.length} of ${value.length})`}
              </Button>
            )}
            {entry && (
              <ValueViewer
                key={`${name}:${picked}:${entry.key?.text ?? entry.value.text}`}
                label={`Value · ${entry.key?.text ?? entry.value.text}`}
                entry={entry}
                score={value.kind === 'zset'}
                editable={
                  !entry.value.binary &&
                  (value.kind === 'hash' || value.kind === 'list' || value.kind === 'zset')
                }
                onSave={(text) => {
                  if (value.kind === 'hash')
                    return edit({ kind: 'setField', field: entry.key!.text, value: text })
                  if (value.kind === 'list') {
                    return edit({
                      kind: 'setItem',
                      index: Number(entry.key!.text),
                      old: entry.value.text,
                      value: text,
                    })
                  }
                  return edit({ kind: 'addMember', member: entry.value.text, score: parseScore(text) })
                }}
                onRemove={() => {
                  if (value.kind === 'hash') return edit({ kind: 'removeField', field: entry.key!.text })
                  if (value.kind === 'list') {
                    return edit({ kind: 'removeItem', index: Number(entry.key!.text), old: entry.value.text })
                  }
                  if (value.kind === 'stream') return edit({ kind: 'removeEntry', id: entry.key!.text })
                  return edit({ kind: 'removeMember', member: entry.value.text })
                }}
              />
            )}
          </div>
        </>
      )}

      {dialog === 'ttl' && (
        <FormDialog
          title="Set TTL"
          submitLabel="Set TTL"
          fields={[
            {
              name: 'seconds',
              label: 'Expires in (seconds)',
              type: 'number',
              initial: value.ttlMs === null ? '' : String(Math.ceil(value.ttlMs / 1000)),
            },
          ]}
          onSubmit={async ({ seconds }) => {
            const ttl = Number(seconds)
            if (!Number.isFinite(ttl) || ttl <= 0) throw 'Enter a number of seconds greater than 0.'
            await edit({ kind: 'expire', ttlMs: Math.round(ttl * 1000) })
          }}
          secondary={
            value.ttlMs === null
              ? undefined
              : { label: 'Remove TTL', run: () => edit({ kind: 'expire', ttlMs: null }) }
          }
          onClose={() => setDialog(null)}
        />
      )}
      {dialog === 'rename' && (
        <FormDialog
          title="Rename key"
          submitLabel="Rename"
          fields={[{ name: 'to', label: 'New name', initial: name }]}
          onSubmit={async ({ to }) => {
            await keyspaceApi.editKey(connectionId, db, name, value.kind, { kind: 'rename', to })
            onKeysChanged(to)
          }}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog === 'delete' && (
        <FormDialog
          title="Delete key"
          submitLabel="Delete"
          danger
          fields={[]}
          onSubmit={async () => {
            await keyspaceApi.editKey(connectionId, db, name, value.kind, { kind: 'delete' })
            onKeysChanged(null)
          }}
          onClose={() => setDialog(null)}
        >
          <p>
            Delete <strong className={styles.mono}>{name}</strong> and its{' '}
            {value.length.toLocaleString('en-US')} {shape?.noun ?? 'elements'}? This can't be undone.
          </p>
        </FormDialog>
      )}
      {dialog === 'add' && shape?.add && (
        <FormDialog
          title={shape.add}
          submitLabel="Add"
          fields={addFields(value.kind)}
          onSubmit={async (form) => edit(addEdit(value.kind, form))}
          onClose={() => setDialog(null)}
        />
      )}
    </section>
  )
}

function addFields(kind: string): FormField[] {
  switch (kind) {
    case 'hash':
      return [
        { name: 'field', label: 'Field' },
        { name: 'value', label: 'Value', type: 'textarea' },
      ]
    case 'list':
      return [
        { name: 'value', label: 'Value', type: 'textarea' },
        {
          name: 'end',
          label: 'Add at',
          type: 'select',
          initial: 'tail',
          options: [
            { value: 'tail', label: 'The end (RPUSH)' },
            { value: 'head', label: 'The start (LPUSH)' },
          ],
        },
      ]
    case 'zset':
      return [
        { name: 'member', label: 'Member' },
        { name: 'score', label: 'Score', type: 'number' },
      ]
    case 'stream':
      return [
        {
          name: 'fields',
          label: 'Fields, one field=value per line',
          type: 'textarea',
          placeholder: 'event=login',
        },
      ]
    default:
      return [{ name: 'member', label: 'Member' }]
  }
}

function addEdit(kind: string, form: Record<string, string>): KeyEdit {
  switch (kind) {
    case 'hash':
      if (!form.field) throw 'Enter a field name.'
      return { kind: 'setField', field: form.field, value: form.value }
    case 'list':
      return { kind: 'pushItem', value: form.value, head: form.end === 'head' }
    case 'zset':
      return { kind: 'addMember', member: form.member, score: parseScore(form.score) }
    case 'stream':
      return { kind: 'addEntry', fields: parseFields(form.fields) }
    default:
      return { kind: 'addMember', member: form.member, score: null }
  }
}

interface ValueViewerProps {
  label: string
  entry: Entry
  editable: boolean
  /** Sorted sets edit the score, not the member. */
  score?: boolean
  onSave: (text: string) => Promise<void>
  onRemove?: () => Promise<void>
}

/** One value as text, JSON or hex, with editing where the type allows it. */
function ValueViewer({ label, entry, editable, score = false, onSave, onRemove }: ValueViewerProps) {
  const [mode, setMode] = useState<'text' | 'json' | 'hex'>(() =>
    asJson(entry.value.text) ? 'json' : 'text',
  )
  const [draft, setDraft] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const json = asJson(entry.value.text)
  const shown = entry.value.binary
    ? entry.value.text.replace(/(..)/g, '$1 ').trim()
    : mode === 'json' && json
      ? json
      : mode === 'hex'
        ? asHex(entry.value.text)
        : entry.value.text

  const act = async (action: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await action()
      setDraft(null)
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className={styles.viewer}>
      <div className={styles.viewerBar}>
        <span className={styles.label}>{label}</span>
        <span className={styles.spacer} />
        {entry.value.binary ? (
          <Badge>binary · hex</Badge>
        ) : (
          <div className={styles.seg} role="group" aria-label="Show value as">
            <button type="button" aria-pressed={mode === 'text'} onClick={() => setMode('text')}>
              Text
            </button>
            <button
              type="button"
              aria-pressed={mode === 'json'}
              disabled={!json}
              onClick={() => setMode('json')}
            >
              JSON
            </button>
            <button type="button" aria-pressed={mode === 'hex'} onClick={() => setMode('hex')}>
              Hex
            </button>
          </div>
        )}
        {editable && draft === null && (
          <Button onClick={() => setDraft(score ? String(entry.score ?? '') : entry.value.text)}>
            {score ? 'Edit score' : 'Edit'}
          </Button>
        )}
        {onRemove && draft === null && (
          <Button className={styles.danger} disabled={busy} onClick={() => void act(onRemove)}>
            Remove
          </Button>
        )}
      </div>
      {draft === null ? (
        <pre className={styles.value}>{shown}</pre>
      ) : (
        <form
          className={styles.editor}
          onSubmit={(event) => {
            event.preventDefault()
            void act(() => onSave(draft))
          }}
        >
          <textarea
            className={`${styles.input} ${styles.textarea} ${styles.mono}`}
            value={draft}
            rows={score ? 1 : 6}
            aria-label={score ? 'Score' : 'Value'}
            autoFocus
            spellCheck={false}
            onChange={(event) => setDraft(event.target.value)}
          />
          <div className={styles.editorActions}>
            <Button onClick={() => setDraft(null)}>Cancel</Button>
            <Button type="submit" variant="primary" disabled={busy}>
              Save
            </Button>
          </div>
        </form>
      )}
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
    </div>
  )
}
