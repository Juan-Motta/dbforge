import { useEffect, useState } from 'react'
import { Badge } from '../../components/Badge'
import { Button, IconButton } from '../../components/Button'
import { ContextMenu, type MenuItem } from '../../components/ContextMenu'
import { Icon } from '../../components/Icon'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { keyspaceApi, type Entry, type KeyEdit, type KeyValue } from './api'
import { FormDialog, type FormField } from './FormDialog'
import {
  asHex,
  asJson,
  asMsgpack,
  formatBytes,
  formatTtl,
  kindLabel,
  parseFields,
  parseScore,
} from './keyspace'
import { kindClass } from './kindClass'
import { messages, type KeyspaceMessages } from './messages'
import styles from './KeyBrowser.module.css'

const PAGE = 200

/** How each type names its elements and columns. */
function shapes(
  m: KeyspaceMessages,
): Record<string, { noun: string; add?: string; key?: string; value: string; filter: boolean }> {
  return {
    hash: { noun: m.nounFields, add: m.addField, key: m.field, value: m.value, filter: true },
    list: { noun: m.nounItems, add: m.addItem, key: m.index, value: m.value, filter: false },
    set: { noun: m.nounMembers, add: m.addMember, value: m.member, filter: true },
    zset: { noun: m.nounMembers, add: m.addMember, value: m.member, filter: true },
    stream: { noun: m.nounEntries, add: m.addEntry, key: m.id, value: m.fields, filter: false },
    string: { noun: m.nounBytes, value: m.value, filter: false },
  }
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
  /** A dialog to open at once, asked for from the key list's menu. */
  initialDialog?: 'rename' | 'ttl' | 'delete'
}

/** One key: its type, TTL and size, its value a page at a time, and its edits. */
export function KeyDetail({
  connectionId,
  db,
  name,
  reloadToken,
  onKeysChanged,
  initialDialog,
}: KeyDetailProps) {
  const m = useMessages(messages)
  const [value, setValue] = useState<KeyValue | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [filter, setFilter] = useState('')
  const [loadingMore, setLoadingMore] = useState(false)
  const [picked, setPicked] = useState(0)
  const [dialog, setDialog] = useState<Dialog>(initialDialog ?? null)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)

  /** Removes one field, item, member or stream entry of the key. */
  const removeEntry = (kind: string, entry: Entry) => {
    if (kind === 'hash') return edit({ kind: 'removeField', field: entry.key!.text })
    if (kind === 'list')
      return edit({ kind: 'removeItem', index: Number(entry.key!.text), old: entry.value.text })
    if (kind === 'stream') return edit({ kind: 'removeEntry', id: entry.key!.text })
    return edit({ kind: 'removeMember', member: entry.value.text })
  }

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
  if (!value) return <p className={styles.detailEmpty}>{m.loadingKey(name)}</p>

  const shape = shapes(m)[value.kind]
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
    <section className={styles.detail} aria-label={m.keyRegion(name)}>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      <header className={styles.detailHeader}>
        <span className={`${styles.kind} ${styles.kindLarge} ${kindClass(value.kind)}`}>
          {kindLabel(value.kind)}
        </span>
        <span className={`${styles.mono} ${styles.keyName}`}>{value.name}</span>
        <IconButton label={m.renameKey} onClick={() => setDialog('rename')}>
          <Icon name="edit" />
        </IconButton>
        <span className={styles.spacer} />
        {value.ttlMs !== null && (
          <Badge tone="warn">
            <Icon name="history" size={12} />
            TTL {formatTtl(value.ttlMs, m, true)}
          </Badge>
        )}
        {shape && (
          <Badge>
            {value.length.toLocaleString(m.locale)} {shape.noun}
          </Badge>
        )}
        {value.bytes !== null && <Badge>{formatBytes(value.bytes)}</Badge>}
        {value.encoding && <Badge>{value.encoding}</Badge>}
        <Button onClick={() => setDialog('ttl')}>{m.setTtl}</Button>
        <Button className={styles.danger} onClick={() => setDialog('delete')}>
          {m.delete}
        </Button>
      </header>

      {!shape ? (
        <p className={styles.detailEmpty}>{m.moduleKeys(value.kind)}</p>
      ) : value.kind === 'string' ? (
        <div className={styles.detailBody}>
          {entry && (
            <ValueViewer
              key={name}
              label={m.stringValue(formatBytes(value.length), value.length > entry.size)}
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
                placeholder={m.filterNoun(shape.noun)}
                aria-label={m.filterNoun(shape.noun)}
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
            <div className={styles.entries} role="table" aria-label={m.nounOf(shape.noun, name)}>
              <div className={`${styles.entryRow} ${styles.entryHead}`} role="row">
                <span role="columnheader">#</span>
                {shape.key && <span role="columnheader">{shape.key}</span>}
                <span role="columnheader">{shape.value}</span>
                {value.kind === 'zset' && <span role="columnheader">{m.score}</span>}
                <span role="columnheader" className={styles.right}>
                  {m.size}
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
                  onContextMenu={(event) => {
                    event.preventDefault()
                    setPicked(index)
                    const write = (text: string) => void navigator.clipboard.writeText(text)
                    setMenu({
                      x: event.clientX,
                      y: event.clientY,
                      items: [
                        ...(item.key
                          ? [{ label: m.copyFieldOf(shape.key!), onSelect: () => write(item.key!.text) }]
                          : []),
                        { label: m.copyFieldOf(shape.value), onSelect: () => write(item.value.text) },
                        ...(value.kind === 'zset' && item.score !== null && item.score !== undefined
                          ? [{ label: m.copyFieldOf(m.score), onSelect: () => write(String(item.score)) }]
                          : []),
                        {
                          label: m.remove,
                          icon: 'trash' as const,
                          onSelect: () => void removeEntry(value.kind, item),
                        },
                      ],
                    })
                  }}
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
              {value.entries.length === 0 && <p className={styles.muted}>{m.nothingMatches}</p>}
            </div>
            {value.next && (
              <Button
                variant="ghost"
                className={styles.more}
                disabled={loadingMore}
                onClick={() => void loadMore()}
              >
                {loadingMore ? m.loading : m.loadMoreOf(value.entries.length, value.length)}
              </Button>
            )}
            {entry && (
              <ValueViewer
                key={`${name}:${picked}:${entry.key?.text ?? entry.value.text}`}
                label={m.entryValue(entry.key?.text ?? entry.value.text)}
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
                  return edit({ kind: 'addMember', member: entry.value.text, score: parseScore(text, m) })
                }}
                onRemove={() => removeEntry(value.kind, entry)}
              />
            )}
          </div>
        </>
      )}

      {dialog === 'ttl' && (
        <FormDialog
          title={m.setTtl}
          submitLabel={m.setTtl}
          fields={[
            {
              name: 'seconds',
              label: m.expiresIn,
              type: 'number',
              initial: value.ttlMs === null ? '' : String(Math.ceil(value.ttlMs / 1000)),
            },
          ]}
          onSubmit={async ({ seconds }) => {
            const ttl = Number(seconds)
            if (!Number.isFinite(ttl) || ttl <= 0) throw m.secondsGreaterThanZero
            await edit({ kind: 'expire', ttlMs: Math.round(ttl * 1000) })
          }}
          secondary={
            value.ttlMs === null
              ? undefined
              : { label: m.removeTtl, run: () => edit({ kind: 'expire', ttlMs: null }) }
          }
          onClose={() => setDialog(null)}
        />
      )}
      {dialog === 'rename' && (
        <FormDialog
          title={m.renameKey}
          submitLabel={m.rename}
          fields={[{ name: 'to', label: m.newName, initial: name }]}
          onSubmit={async ({ to }) => {
            await keyspaceApi.editKey(connectionId, db, name, value.kind, { kind: 'rename', to })
            onKeysChanged(to)
          }}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog === 'delete' && (
        <FormDialog
          title={m.deleteKey}
          submitLabel={m.delete}
          danger
          fields={[]}
          onSubmit={async () => {
            await keyspaceApi.editKey(connectionId, db, name, value.kind, { kind: 'delete' })
            onKeysChanged(null)
          }}
          onClose={() => setDialog(null)}
        >
          <p>
            {m.deleteBefore} <strong className={styles.mono}>{name}</strong>
            {m.deleteAfter(value.length.toLocaleString(m.locale), shape?.noun ?? m.nounElements)}
          </p>
        </FormDialog>
      )}
      {dialog === 'add' && shape?.add && (
        <FormDialog
          title={shape.add}
          submitLabel={m.add}
          fields={addFields(value.kind, m)}
          onSubmit={async (form) => edit(addEdit(value.kind, form, m))}
          onClose={() => setDialog(null)}
        />
      )}
    </section>
  )
}

function addFields(kind: string, m: KeyspaceMessages): FormField[] {
  switch (kind) {
    case 'hash':
      return [
        { name: 'field', label: m.field },
        { name: 'value', label: m.value, type: 'textarea' },
      ]
    case 'list':
      return [
        { name: 'value', label: m.value, type: 'textarea' },
        {
          name: 'end',
          label: m.addAt,
          type: 'select',
          initial: 'tail',
          options: [
            { value: 'tail', label: m.theEnd },
            { value: 'head', label: m.theStart },
          ],
        },
      ]
    case 'zset':
      return [
        { name: 'member', label: m.member },
        { name: 'score', label: m.score, type: 'number' },
      ]
    case 'stream':
      return [
        {
          name: 'fields',
          label: m.fieldsPerLine,
          type: 'textarea',
          placeholder: 'event=login',
        },
      ]
    default:
      return [{ name: 'member', label: m.member }]
  }
}

function addEdit(kind: string, form: Record<string, string>, m: KeyspaceMessages): KeyEdit {
  switch (kind) {
    case 'hash':
      if (!form.field) throw m.enterFieldName
      return { kind: 'setField', field: form.field, value: form.value }
    case 'list':
      return { kind: 'pushItem', value: form.value, head: form.end === 'head' }
    case 'zset':
      return { kind: 'addMember', member: form.member, score: parseScore(form.score, m) }
    case 'stream':
      return { kind: 'addEntry', fields: parseFields(form.fields, m) }
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
  const m = useMessages(messages)
  const json = asJson(entry.value.text)
  const msgpack = asMsgpack(entry.value)
  const binary = entry.value.binary
  // Binary values have no text to show: MessagePack when they decode, else hex.
  const [mode, setMode] = useState<'text' | 'json' | 'hex' | 'msgpack'>(() =>
    binary ? (msgpack ? 'msgpack' : 'hex') : json ? 'json' : 'text',
  )
  const [draft, setDraft] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const shown =
    mode === 'msgpack' && msgpack
      ? msgpack
      : binary
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
        <div className={styles.seg} role="group" aria-label={m.showValueAs}>
          <button
            type="button"
            aria-pressed={mode === 'text'}
            disabled={binary}
            onClick={() => setMode('text')}
          >
            {m.text}
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
          <button
            type="button"
            aria-pressed={mode === 'msgpack'}
            disabled={!msgpack}
            title={binary ? undefined : m.msgpackBinary}
            onClick={() => setMode('msgpack')}
          >
            MsgPack
          </button>
        </div>
        {editable && draft === null && (
          <Button onClick={() => setDraft(score ? String(entry.score ?? '') : entry.value.text)}>
            {score ? m.editScore : m.edit}
          </Button>
        )}
        {onRemove && draft === null && (
          <Button className={styles.danger} disabled={busy} onClick={() => void act(onRemove)}>
            {m.remove}
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
            aria-label={score ? m.score : m.value}
            autoFocus
            spellCheck={false}
            onChange={(event) => setDraft(event.target.value)}
          />
          <div className={styles.editorActions}>
            <Button onClick={() => setDraft(null)}>{m.cancel}</Button>
            <Button type="submit" variant="primary" disabled={busy}>
              {m.save}
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
