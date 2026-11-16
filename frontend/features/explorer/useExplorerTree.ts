import { useCallback, useMemo, useState } from 'react'
import type { Connection } from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { explorerApi, type DbObject } from './api'
import { messages } from './messages'
import { connectionNode, loadsChildren, nodesToReload, toChildNodes, type TreeNode } from './tree'

// The listed objects are kept, not their nodes, so folder names follow the language.
type ChildState =
  { status: 'loading' } | { status: 'loaded'; objects: DbObject[] } | { status: 'error'; message: string }

/**
 * The explorer tree for the given connections. A node's children load from the
 * backend the first time it is expanded; `refresh` reloads every expanded node,
 * or with a node, only that node's part of the tree.
 */
export function useExplorerTree(connections: Connection[]) {
  const m = useMessages(messages)
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
  const [children, setChildren] = useState<Record<string, ChildState>>({})

  const roots = useMemo(() => {
    const attach = (node: TreeNode): TreeNode => {
      if (node.children) return { ...node, children: node.children.map(attach) }
      const state = children[node.id]
      if (!state) return node
      if (state.status === 'loading') return { ...node, loading: true }
      if (state.status === 'error') return { ...node, error: state.message }
      return { ...node, children: toChildNodes(node, state.objects, m).map(attach) }
    }
    return connections.map((c) => attach(connectionNode(c)))
  }, [connections, children, m])

  const load = useCallback((nodes: TreeNode[]) => {
    setChildren((current) => {
      const next = { ...current }
      nodes.forEach((node) => (next[node.id] = { status: 'loading' }))
      return next
    })
    nodes.forEach(async (node) => {
      let state: ChildState
      try {
        state = { status: 'loaded', objects: await explorerApi.listObjects(node.connectionId, node.path) }
      } catch (error) {
        state = { status: 'error', message: errorMessage(error) }
      }
      setChildren((current) => ({ ...current, [node.id]: state }))
    })
  }, [])

  const toggle = (node: TreeNode) => {
    const expanding = !expanded.has(node.id)
    setExpanded((current) => {
      const next = new Set(current)
      if (expanding) next.add(node.id)
      else next.delete(node.id)
      return next
    })
    // Load on first expand, and retry after an error.
    const state = children[node.id]
    if (expanding && loadsChildren(node) && (!state || state.status === 'error')) load([node])
  }

  const collapseAll = () => setExpanded(new Set())

  const refresh = (scope?: TreeNode) => load(nodesToReload(roots, (id) => expanded.has(id), scope?.id))

  return { roots, expanded, toggle, collapseAll, refresh }
}
