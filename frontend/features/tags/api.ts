import { invoke } from '@tauri-apps/api/core'
import type { Tag } from './tags'

export const tagsApi = {
  list: () => invoke<Tag[]>('list_tags'),
  /** Saves the whole list, in order; connections lose the tags it no longer has. */
  save: (list: Tag[]) => invoke<Tag[]>('save_tags', { list }),
}
