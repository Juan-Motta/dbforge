import { useEffect, useState } from 'react'
import { tagsApi } from './api'
import type { Tag } from './tags'
import type { TagsState } from './useTags'

/**
 * The tags, loaded once and kept in step with the backend. `onRemoved` runs
 * after a save deleted a tag: the connections that had it changed too.
 */
export function useTagList(onRemoved: () => void): TagsState {
  const [tags, setTags] = useState<Tag[] | null>(null)

  useEffect(() => {
    tagsApi
      .list()
      .then(setTags)
      .catch(() => {})
  }, [])

  const save = async (next: Tag[]) => {
    const removed = (tags ?? []).some((tag) => !next.some((kept) => kept.id === tag.id))
    setTags(await tagsApi.save(next))
    if (removed) onRemoved()
  }

  return { tags: tags ?? [], loaded: tags !== null, save }
}
