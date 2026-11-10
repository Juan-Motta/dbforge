/** A palette that reads in both themes; `styles/theme.css` holds each color per theme. */
export const TAG_COLORS = [
  'red',
  'orange',
  'amber',
  'green',
  'teal',
  'blue',
  'violet',
  'pink',
  'gray',
] as const

export type TagColor = (typeof TAG_COLORS)[number]

/** What a connection can carry as its environment. */
export interface Tag {
  /** What connections store; it never changes when the tag is renamed. */
  id: string
  name: string
  color: TagColor
  /** Changing data on a connection with this tag asks first, as production does. */
  confirm: boolean
}

/** The longest name a badge has room for; the backend refuses longer ones. */
export const MAX_TAG_NAME = 16

/** The CSS color of a palette entry, for the current theme. */
export function tagColor(color: TagColor): string {
  return `var(--tag-${color})`
}

/** The tag a connection carries, if it still exists. */
export function tagOf(tags: Tag[], id: string | null): Tag | null {
  return (id && tags.find((tag) => tag.id === id)) || null
}

/** The name of the tag that makes changes on a connection tagged `id` ask first; `null` when they don't. */
export function confirmingTag(tags: Tag[], id: string | null): string | null {
  const tag = tagOf(tags, id)
  return tag?.confirm ? tag.name : null
}

/** A new tag, with the first color no other tag uses yet. */
export function newTag(tags: Tag[], name: string, id: string): Tag {
  const used = new Set(tags.map((tag) => tag.color))
  return { id, name, color: TAG_COLORS.find((color) => !used.has(color)) ?? 'gray', confirm: false }
}
