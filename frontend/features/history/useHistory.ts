import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { historyApi, type HistoryEntry } from './api'

type Result = { key: string } & ({ entries: HistoryEntry[]; loadedAt: number } | { error: string })

/** Loads matching history entries, again whenever the filters or `reloadKey` change, or on `reload()`. */
export function useHistory(search: string, errorsOnly: boolean, reloadKey?: unknown) {
  const [reloads, setReloads] = useState(0)
  const [result, setResult] = useState<Result | null>(null)
  const key = JSON.stringify([search, errorsOnly, reloads, reloadKey ?? null])

  useEffect(() => {
    const request = { cancelled: false }
    historyApi
      .list(search, errorsOnly)
      .then((entries) => !request.cancelled && setResult({ key, entries, loadedAt: Date.now() }))
      .catch((error) => !request.cancelled && setResult({ key, error: errorMessage(error) }))
    return () => {
      request.cancelled = true
    }
  }, [search, errorsOnly, key])

  const loading = result?.key !== key
  return {
    // Keep showing the previous list while the next one loads, so typing doesn't flicker.
    entries: result && 'entries' in result ? result.entries : null,
    /** When the list arrived; "Today" and "Yesterday" are relative to it. */
    loadedAt: result && 'loadedAt' in result ? result.loadedAt : 0,
    error: !loading && result && 'error' in result ? result.error : null,
    loading,
    reload: () => setReloads((current) => current + 1),
  }
}
