import { useEffect, useState } from 'react'
import { getCurrentWindow, UserAttentionType } from '@tauri-apps/api/window'
import { mcpApi, type ApprovalRequest } from './api'

/**
 * Changes agents are waiting to make, oldest first; the first is the one the
 * dialog shows. Answering it removes it, whatever the backend says, since a
 * request that already timed out has nothing left to wait for.
 */
export function useApprovals() {
  const [waiting, setWaiting] = useState<ApprovalRequest[]>([])

  useEffect(() => {
    let current = true
    const stop = mcpApi.onApproval((request) => {
      if (!current) return
      setWaiting((state) => [...state, request])
      // The user is likely in the agent's terminal: bring the question to them.
      const window = getCurrentWindow()
      void window.setFocus().catch(() => {})
      void window.requestUserAttention(UserAttentionType.Critical).catch(() => {})
    })
    return () => {
      current = false
      void stop.then((unlisten) => unlisten())
    }
  }, [])

  return {
    current: waiting[0] ?? null,
    answer: (id: string, approved: boolean) => {
      setWaiting((state) => state.filter((request) => request.id !== id))
      void mcpApi.answer(id, approved).catch(() => {})
    },
  }
}
