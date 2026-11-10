import { useMemo, useState } from 'react'
import { Button, IconButton } from '../../components/Button'
import { ContextMenu, type MenuItem } from '../../components/ContextMenu'
import { Icon } from '../../components/Icon'
import type { Connection } from '../../domain/connection'
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import { filterTree, visibleRows, type TreeNode } from './tree'
import { INDENT_PX, TreeItem } from './TreeItem'
import { useExplorerTree } from './useExplorerTree'
import styles from './Explorer.module.css'

interface ExplorerProps {
  connections: Connection[]
  onSelect: (node: TreeNode) => void
  /** Double-click on a table or view. */
  onOpen: (node: TreeNode) => void
  /** Opens the ER diagram of a schema node; `null` when the node has none. */
  diagramOf: (node: TreeNode) => (() => void) | null
  onNewConnection: () => void
  /** A node's right-click menu; `refresh` reloads the tree, e.g. once a database was created. */
  menuOf: (node: TreeNode, refresh: () => void) => MenuItem[]
}

export function Explorer({
  connections,
  onSelect,
  onOpen,
  diagramOf,
  onNewConnection,
  menuOf,
}: ExplorerProps) {
  const m = useMessages(messages)
  const { roots, expanded, toggle, collapseAll, refresh } = useExplorerTree(connections)
  const [query, setQuery] = useState('')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)

  const rows = useMemo(() => {
    const filtering = query.trim() !== ''
    // While filtering, every loaded match is shown expanded so it is visible.
    return visibleRows(filterTree(roots, query), (id) => filtering || expanded.has(id), m)
  }, [roots, query, expanded, m])

  const select = (node: TreeNode) => {
    setSelectedId(node.id)
    onSelect(node)
  }

  return (
    <aside className={`island ${styles.explorer}`} aria-label={m.explorer}>
      <div className={styles.header}>
        <span className={styles.title}>{m.explorer}</span>
        <IconButton label={m.newConnection} onClick={onNewConnection}>
          <Icon name="plus" />
        </IconButton>
        <IconButton label={m.refresh} onClick={refresh}>
          <Icon name="refresh" />
        </IconButton>
        <IconButton label={m.collapseAll} onClick={collapseAll}>
          <Icon name="collapse" />
        </IconButton>
      </div>

      <label className={styles.filter}>
        <Icon name="search" className={styles.filterIcon} />
        <input
          type="search"
          placeholder={m.filterPlaceholder}
          aria-label={m.filter}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>

      <div role="tree" aria-label={m.connections} className={styles.tree}>
        {rows.map((row) =>
          row.type === 'node' ? (
            <TreeItem
              key={row.node.id}
              node={row.node}
              depth={row.depth}
              expanded={row.expanded}
              selected={row.node.id === selectedId}
              onSelect={select}
              onToggle={toggle}
              onOpen={onOpen}
              onDiagram={diagramOf(row.node) ?? undefined}
              onMenu={(node, x, y) => {
                const items = menuOf(node, refresh)
                setMenu(items.length > 0 ? { x, y, items } : null)
              }}
            />
          ) : (
            <p
              key={row.id}
              className={row.tone === 'error' ? styles.error : styles.message}
              style={{ paddingLeft: 24 + row.depth * INDENT_PX }}
            >
              {row.text}
            </p>
          ),
        )}

        {connections.length === 0 && (
          <div className={styles.empty}>
            <p>{m.noConnections}</p>
            <Button variant="primary" onClick={onNewConnection}>
              <Icon name="plus" />
              {m.newConnection}
            </Button>
          </div>
        )}
        {connections.length > 0 && rows.length === 0 && (
          <p className={styles.message}>{m.noMatches(query.trim())}</p>
        )}
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </aside>
  )
}
