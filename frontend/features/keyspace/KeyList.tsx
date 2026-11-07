import { useEffect, useState } from 'react'
import { Button, IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { errorMessage } from '../../lib/errors'
import { keyspaceApi, type KeyInfo } from './api'
import { formatTtl, keyTree, kindLabel, visibleKeyRows } from './keyspace'
import { kindClass } from './kindClass'
import styles from './KeyBrowser.module.css'

const PAGE = 500
const KINDS = ['string', 'hash', 'list', 'set', 'zset', 'stream']

interface KeyListProps {
  connectionId: string
  db: number
  selected: string | null
  onSelect: (key: KeyInfo) => void
  onAddKey: () => void
  /** Changes when keys were added, renamed or deleted, to scan again. */
  reloadToken: number
}

/** The keys of a database found with SCAN, as a `:` tree or a flat list. */
export function KeyList({ connectionId, db, selected, onSelect, onAddKey, reloadToken }: KeyListProps) {
  const [draft, setDraft] = useState('')
  const [pattern, setPattern] = useState('')
  const [kind, setKind] = useState('')
  const [view, setView] = useState<'tree' | 'list'>('tree')
  const [keys, setKeys] = useState<KeyInfo[]>([])
  const [cursor, setCursor] = useState('0')
  const [state, setState] = useState<{ loading: boolean; error: string | null }>({
    loading: true,
    error: null,
  })
  const [expanded, setExpanded] = useState<Set<string>>(new Set())

  const load = async (from: string, append: boolean) => {
    setState({ loading: true, error: null })
    try {
      const page = await keyspaceApi.scan(connectionId, db, pattern, kind || null, from, PAGE)
      setKeys((current) => (append ? [...current, ...page.keys] : page.keys))
      setCursor(page.cursor)
      setState({ loading: false, error: null })
    } catch (error) {
      setState({ loading: false, error: errorMessage(error) })
    }
  }

  useEffect(() => {
    void load('0', false)
    // `load` reads the values these name.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectionId, db, pattern, kind, reloadToken])

  const tree = keyTree(keys)
  const rows =
    view === 'tree'
      ? visibleKeyRows(tree, (prefix) => expanded.has(prefix))
      : keys.map((key) => ({ node: { kind: 'key' as const, label: key.name, key }, depth: 0 }))
  const toggle = (prefix: string) =>
    setExpanded((current) => {
      const next = new Set(current)
      if (next.has(prefix)) next.delete(prefix)
      else next.add(prefix)
      return next
    })

  return (
    <aside className={styles.keys} aria-label="Keys">
      <form
        className={styles.keysBar}
        onSubmit={(event) => {
          event.preventDefault()
          setPattern(draft.trim())
        }}
      >
        <label className={styles.search}>
          <span className={styles.srOnly}>Key pattern</span>
          <Icon name="search" size={14} />
          <input
            className={`${styles.input} ${styles.mono}`}
            value={draft}
            placeholder="Pattern, e.g. session:*"
            spellCheck={false}
            onChange={(event) => setDraft(event.target.value)}
            onBlur={() => setPattern(draft.trim())}
          />
        </label>
        <select
          className={`${styles.input} ${styles.kindSelect}`}
          aria-label="Key type"
          value={kind}
          onChange={(event) => setKind(event.target.value)}
        >
          <option value="">All types</option>
          {KINDS.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
        <IconButton label="Add key" onClick={onAddKey}>
          <Icon name="plus" />
        </IconButton>
      </form>

      <div className={styles.scanInfo}>
        <span>
          {cursor === '0' ? 'SCAN complete' : `SCAN cursor ${cursor}`} · {keys.length.toLocaleString('en-US')}{' '}
          matched
        </span>
        <span className={styles.spacer} />
        <div className={styles.seg} role="group" aria-label="Key view">
          <button type="button" aria-pressed={view === 'tree'} onClick={() => setView('tree')}>
            Tree
          </button>
          <button type="button" aria-pressed={view === 'list'} onClick={() => setView('list')}>
            List
          </button>
        </div>
      </div>

      <div className={styles.keyRows} role="tree" aria-label="Keys">
        {state.error && <p className={styles.error}>{state.error}</p>}
        {!state.error && !state.loading && keys.length === 0 && (
          <p className={styles.muted}>No keys match.</p>
        )}
        {rows.map(({ node, depth }) =>
          node.kind === 'folder' ? (
            <button
              key={'folder:' + node.prefix}
              type="button"
              role="treeitem"
              aria-expanded={expanded.has(node.prefix)}
              className={styles.folder}
              style={{ paddingLeft: 8 + depth * 14 }}
              onClick={() => toggle(node.prefix)}
            >
              <Icon name={expanded.has(node.prefix) ? 'chevronDown' : 'chevronRight'} size={12} />
              <span className={styles.ellipsis}>
                {depth > 0 ? node.label.slice(lastPrefix(node.label)) : node.label}
              </span>
              <span className={styles.count}>{node.count.toLocaleString('en-US')}</span>
            </button>
          ) : (
            <button
              key={'key:' + node.key.name}
              type="button"
              role="treeitem"
              aria-selected={node.key.name === selected}
              className={styles.key}
              style={{ paddingLeft: 8 + depth * 14 }}
              onClick={() => onSelect(node.key)}
            >
              <span className={`${styles.kind} ${kindClass(node.key.kind)}`}>{kindLabel(node.key.kind)}</span>
              <span className={styles.ellipsis}>
                {view === 'tree' && depth > 0 ? node.label.slice(lastPrefix(node.label)) : node.label}
              </span>
              <span className={styles.ttl}>{formatTtl(node.key.ttlMs)}</span>
            </button>
          ),
        )}
        {cursor !== '0' && (
          <Button
            variant="ghost"
            className={styles.more}
            disabled={state.loading}
            onClick={() => void load(cursor, true)}
          >
            {state.loading ? 'Scanning…' : 'Load more'}
          </Button>
        )}
      </div>
    </aside>
  )
}

/** Where the last segment of a nested name starts: `a:b:c` → index of `c`. */
function lastPrefix(name: string): number {
  const trimmed = name.endsWith(':') ? name.slice(0, -1) : name
  return trimmed.lastIndexOf(':') + 1
}
