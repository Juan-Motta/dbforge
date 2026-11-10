import { createContext, useContext } from 'react'
import type { Tag } from './tags'

export interface TagsState {
  tags: Tag[]
  /** False until the backend answered; changes then ask on any tagged connection. */
  loaded: boolean
  /** Saves the whole list; rejects with the backend's reason when it refuses it. */
  save: (tags: Tag[]) => Promise<void>
}

/** Provided by `App` from `useTagList`; no tags outside it. */
export const TagsContext = createContext<TagsState>({ tags: [], loaded: false, save: async () => {} })

export function useTags(): TagsState {
  return useContext(TagsContext)
}
