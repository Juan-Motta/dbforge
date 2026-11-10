import { createContext, useContext } from 'react'
import type { Tag } from './tags'

export interface TagsState {
  tags: Tag[]
  /** Saves the whole list; rejects with the backend's reason when it refuses it. */
  save: (tags: Tag[]) => Promise<void>
}

/** Provided by `App` from `useTagList`; no tags outside it. */
export const TagsContext = createContext<TagsState>({ tags: [], save: async () => {} })

export function useTags(): TagsState {
  return useContext(TagsContext)
}
