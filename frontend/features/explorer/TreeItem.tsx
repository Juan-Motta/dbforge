import { Badge } from '../../components/Badge'
import { EngineLogo } from '../../components/EngineLogo'
import { Icon, type IconName } from '../../components/Icon'
import type { Environment } from '../../domain/connection'
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import type { ObjectKind, TreeNode } from './tree'
import styles from './Explorer.module.css'

const KIND_ICONS: Record<ObjectKind, IconName> = {
  database: 'database',
  schema: 'schema',
  folder: 'folder',
  table: 'table',
  view: 'view',
}

const ENV_BADGES: Record<Environment, { label: string; tone: 'err' | 'warn' | 'info' }> = {
  production: { label: 'PROD', tone: 'err' },
  staging: { label: 'STAGE', tone: 'warn' },
  development: { label: 'DEV', tone: 'info' },
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
  const m = useMessages(messages)
  const isConnection = node.kind === 'connection'
  const environment = isConnection ? node.connection.environment : null

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
      {environment && <Badge tone={ENV_BADGES[environment].tone}>{ENV_BADGES[environment].label}</Badge>}
      {isConnection && node.children && <span className={styles.liveDot} title={m.connected} />}
      {node.meta && <span className={styles.meta}>{node.meta}</span>}
    </div>
  )
}
