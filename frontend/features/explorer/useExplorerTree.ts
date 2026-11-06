import { useCallback, useMemo, useState } from 'react'
import type { Connection } from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import { explorerApi } from './api'
import { connectionNode, toChildNodes, type TreeNode } from './tree'

type ChildState =
  { status: 'loading' } | { status: 'loaded'; nodes: TreeNode[] } | { status: 'error'; message: string }

/** A node whose children come from the backend (folders arrive pre-loaded). */
const loadsChildren = (node: TreeNode) => node.expandable && node.kind !== 'folder'

/**
 * The explorer tree for the given connections. A node's children load from the
 * backend the first time it is expanded; `refresh` reloads every expanded node.
 */
export function useExplorerTree(connections: Connection[]) {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
  const [children, setChildren] = useState<Record<string, ChildState>>({})

  const roots = useMemo(() => {
    const attach = (node: TreeNode): TreeNode => {
      if (node.children) return { ...node, children: node.children.map(attach) }
      const state = children[node.id]
      if (!state) return node
      if (state.status === 'loading') return { ...node, loading: true }
      if (state.status === 'error') return { ...node, error: state.message }
      return { ...node, children: state.nodes.map(attach) }
    }
    return connections.map((c) => attach(connectionNode(c)))
  }, [connections, children])

  const load = useCallback((nodes: TreeNode[]) => {
    setChildren((current) => {
      const next = { ...current }
      nodes.forEach((node) => (next[node.id] = { status: 'loading' }))
      return next
    })
    nodes.forEach(async (node) => {
      let state: ChildState
      try {
        const objects = await explorerApi.listObjects(node.connectionId, node.path)
        state = { status: 'loaded', nodes: toChildNodes(node, objects) }
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

  const refresh = () => {
    const toReload: TreeNode[] = []
    const walk = (nodes: TreeNode[]) =>
      nodes.forEach((node) => {
        if (!expanded.has(node.id)) return
        if (loadsChildren(node)) toReload.push(node)
        walk(node.children ?? [])
      })
    walk(roots)
    load(toReload)
  }

  return { roots, expanded, toggle, collapseAll, refresh }
}
