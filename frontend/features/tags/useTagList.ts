import { useCallback, useEffect, useMemo, useState } from 'react'
import { tagsApi } from './api'
import type { Tag } from './tags'
import type { TagsState } from './useTags'

/**
 * The tags, loaded once and kept in step with the backend. `onRemoved` runs
 * after a save deleted a tag: the connections that had it changed too. It
 * must be the same function between renders.
 */
export function useTagList(onRemoved: () => void): TagsState {
  const [tags, setTags] = useState<Tag[] | null>(null)

  const reload = useCallback(
    () =>
      tagsApi
        .list()
        .then(setTags)
        .catch(() => {}),
    [],
  )

  useEffect(() => {
    void reload()
  }, [reload])

  // The same object until the tags change: it is a context's value, and a new
  // one would draw everything that reads the tags again.
  return useMemo(() => {
    const save = async (next: Tag[]) => {
      const removed = (tags ?? []).some((tag) => !next.some((kept) => kept.id === tag.id))
      setTags(await tagsApi.save(next))
      if (removed) onRemoved()
    }
    return { tags: tags ?? [], loaded: tags !== null, save, reload }
  }, [tags, reload, onRemoved])
}
