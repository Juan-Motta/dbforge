import { useEffect, useState } from 'react'
import type { Connection } from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import { connectionsApi, type ConnectionInput } from './api'

/** Saved connections, kept in sync with the backend. */
export function useConnections() {
  const [connections, setConnections] = useState<Connection[]>([])
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    connectionsApi
      .list()
      .then(setConnections)
      .catch((error) => setLoadError(errorMessage(error)))
  }, [])

  const save = async (input: ConnectionInput) => {
    const saved = await connectionsApi.save(input)
    setConnections((current) =>
      current.some((c) => c.id === saved.id)
        ? current.map((c) => (c.id === saved.id ? saved : c))
        : [...current, saved],
    )
    return saved
  }

  const remove = async (id: string) => {
    await connectionsApi.remove(id)
    setConnections((current) => current.filter((c) => c.id !== id))
  }

  return { connections, loadError, save, remove }
}
