import { EngineLogo } from '../../components/EngineLogo'
import { Icon, type IconName } from '../../components/Icon'
import { TagBadge } from '../tags/TagBadge'
import { tagOf } from '../tags/tags'
import { useTags } from '../tags/useTags'
import type { ObjectKind, TreeNode } from './tree'
import styles from './Explorer.module.css'

const KIND_ICONS: Record<ObjectKind, IconName> = {
  database: 'database',
  schema: 'schema',
  folder: 'folder',
  table: 'table',
  view: 'view',
}

export const INDENT_PX = 12

interface TreeItemProps {
  node: TreeNode
  depth: number
  expanded: boolean
  selected: boolean
  onSelect: (node: TreeNode) => void
  onToggle: (node: TreeNode) => void
  onOpen: (node: TreeNode) => void
  /** Opens the node's right-click menu at the pointer. */
  onMenu: (node: TreeNode, x: number, y: number) => void
}

export function TreeItem({
  node,
  depth,
  expanded,
  selected,
  onSelect,
  onToggle,
  onOpen,
  onMenu,
}: TreeItemProps) {
  const isConnection = node.kind === 'connection'
  const { tags } = useTags()
  const tag = isConnection ? tagOf(tags, node.connection.environment) : null

  return (
    <div
      role="treeitem"
      aria-label={node.name}
      aria-level={depth + 1}
      aria-expanded={node.expandable ? expanded : undefined}
      aria-selected={selected}
      tabIndex={-1}
      className={styles.item}
      style={{ paddingLeft: 6 + depth * INDENT_PX }}
      onClick={() => onSelect(node)}
      onDoubleClick={() => (node.expandable ? onToggle(node) : onOpen(node))}
      onContextMenu={(event) => {
        event.preventDefault()
        onSelect(node)
        onMenu(node, event.clientX, event.clientY)
      }}
    >
      <span
        className={styles.chevron}
        onClick={(event) => {
          event.stopPropagation()
          if (node.expandable) onToggle(node)
        }}
      >
        {node.expandable && <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} />}
      </span>

      {isConnection ? (
        <EngineLogo engine={node.connection.engine} />
      ) : (
        <span className={styles.kind}>
          <Icon name={KIND_ICONS[node.kind]} size={15} />
        </span>
      )}

      <span className={styles.name}>{node.name}</span>
      {tag && <TagBadge tag={tag} />}
      {node.meta && <span className={styles.meta}>{node.meta}</span>}
    </div>
  )
}
