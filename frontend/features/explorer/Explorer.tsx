import { useMemo, useState } from 'react'
import { Button, IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import type { Connection } from '../../domain/connection'
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
  onEditConnection: (connection: Connection) => void
}

export function Explorer({
  connections,
  onSelect,
  onOpen,
  diagramOf,
  onNewConnection,
  onEditConnection,
}: ExplorerProps) {
  const { roots, expanded, toggle, collapseAll, refresh } = useExplorerTree(connections)
  const [query, setQuery] = useState('')
  const [selectedId, setSelectedId] = useState<string | null>(null)

  const rows = useMemo(() => {
    const filtering = query.trim() !== ''
    // While filtering, every loaded match is shown expanded so it is visible.
    return visibleRows(filterTree(roots, query), (id) => filtering || expanded.has(id))
  }, [roots, query, expanded])

  const select = (node: TreeNode) => {
    setSelectedId(node.id)
    onSelect(node)
  }

  return (
    <aside className={`island ${styles.explorer}`} aria-label="Database explorer">
      <div className={styles.header}>
        <span className={styles.title}>Database explorer</span>
        <IconButton label="New connection" onClick={onNewConnection}>
          <Icon name="plus" />
        </IconButton>
        <IconButton label="Refresh" onClick={refresh}>
          <Icon name="refresh" />
        </IconButton>
        <IconButton label="Collapse all" onClick={collapseAll}>
          <Icon name="collapse" />
        </IconButton>
      </div>

      <label className={styles.filter}>
        <Icon name="search" className={styles.filterIcon} />
        <input
          type="search"
          placeholder="Filter objects…"
          aria-label="Filter objects"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>

      <div role="tree" aria-label="Connections" className={styles.tree}>
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
              onEdit={(node) => node.kind === 'connection' && onEditConnection(node.connection)}
              onDiagram={diagramOf(row.node) ?? undefined}
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
            <p>No connections yet.</p>
            <Button variant="primary" onClick={onNewConnection}>
              <Icon name="plus" />
              New connection
            </Button>
          </div>
        )}
        {connections.length > 0 && rows.length === 0 && (
          <p className={styles.message}>No objects match “{query.trim()}”.</p>
        )}
      </div>
    </aside>
  )
}
