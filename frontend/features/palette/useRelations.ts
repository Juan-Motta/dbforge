import { useEffect, useState } from 'react'
import { errorMessage } from '../../lib/errors'
import { paletteApi, type RelationList } from './api'

type Result = { key: string } & ({ list: RelationList } | { error: string })

/** Loads the tables and views to search; nothing when there is no connection. */
export function useRelations(connectionId?: string, database?: string) {
  const [result, setResult] = useState<Result | null>(null)
  const key = JSON.stringify([connectionId, database])

  useEffect(() => {
    if (!connectionId) return
    const request = { cancelled: false }
    paletteApi
      .listRelations(connectionId, database)
      .then((list) => !request.cancelled && setResult({ key, list }))
      .catch((error) => !request.cancelled && setResult({ key, error: errorMessage(error) }))
    return () => {
      request.cancelled = true
    }
  }, [connectionId, database, key])

  const current = result?.key === key ? result : null
  return {
    list: current && 'list' in current ? current.list : null,
    error: current && 'error' in current ? current.error : null,
    loading: Boolean(connectionId) && !current,
  }
}
