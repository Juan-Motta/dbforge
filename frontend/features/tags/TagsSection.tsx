import { useState, type KeyboardEvent } from 'react'
import { Button, IconButton } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { errorMessage } from '../../lib/errors'
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import { MAX_TAG_NAME, newTag, TAG_COLORS, tagColor, type Tag, type TagColor } from './tags'
import { useTags } from './useTags'
import styles from './TagsSection.module.css'

/**
 * The settings' list of tags. Colors, the confirmation switch, adding and
 * deleting save at once; a name saves on Enter or when the field is left.
 */
export function TagsSection() {
  const m = useMessages(messages)
  const { tags, save } = useTags()
  // Names being typed, by tag id, until they are saved.
  const [names, setNames] = useState<Record<string, string>>({})
  const [picking, setPicking] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  // A tag just added gets the focus, ready to be named.
  const [added, setAdded] = useState<string | null>(null)

  const withNames = (list: Tag[]) => list.map((tag) => ({ ...tag, name: names[tag.id] ?? tag.name }))

  const commit = async (next: Tag[]) => {
    setError(null)
    try {
      await save(next)
      setNames({})
    } catch (failure) {
      setError(errorMessage(failure))
    }
  }

  const change = (id: string, changes: Partial<Tag>) =>
    void commit(withNames(tags).map((tag) => (tag.id === id ? { ...tag, ...changes } : tag)))

  const add = () => {
    const taken = new Set(withNames(tags).map((tag) => tag.name.toLowerCase()))
    let n = 1
    while (taken.has(m.newName(n).toLowerCase())) n++
    const tag = newTag(tags, m.newName(n), crypto.randomUUID())
    setAdded(tag.id)
    void commit([...withNames(tags), tag])
  }

  const onNameKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') event.currentTarget.blur()
  }

  return (
    <>
      <h3 className={styles.heading}>{m.tags}</h3>
      <p className={styles.intro}>{m.tagsIntro}</p>
      <ul className={styles.list}>
        {tags.map((tag) => {
          const name = names[tag.id] ?? tag.name
          return (
            <li key={tag.id} className={styles.row}>
              <span className={styles.colorCell}>
                <button
                  type="button"
                  className={styles.swatch}
                  style={{ background: tagColor(tag.color) }}
                  aria-label={m.color(tag.name)}
                  aria-expanded={picking === tag.id}
                  onClick={() => setPicking(picking === tag.id ? null : tag.id)}
                />
                {picking === tag.id && (
                  <Palette
                    current={tag.color}
                    label={m.colorName}
                    onPick={(color) => {
                      setPicking(null)
                      change(tag.id, { color })
                    }}
                    onClose={() => setPicking(null)}
                  />
                )}
              </span>
              <input
                className={styles.name}
                aria-label={m.name(tag.name)}
                value={name}
                maxLength={MAX_TAG_NAME}
                spellCheck={false}
                autoFocus={added === tag.id}
                onFocus={(event) => added === tag.id && event.currentTarget.select()}
                onChange={(event) => setNames({ ...names, [tag.id]: event.target.value })}
                onKeyDown={onNameKey}
                onBlur={() => {
                  setAdded(null)
                  if (name !== tag.name) void commit(withNames(tags))
                }}
              />
              <label className={styles.confirm} title={m.confirmHelp}>
                <input
                  type="checkbox"
                  checked={tag.confirm}
                  onChange={(event) => change(tag.id, { confirm: event.target.checked })}
                />
                {m.confirm}
              </label>
              <IconButton
                label={m.remove(tag.name)}
                onClick={() => void commit(withNames(tags).filter((other) => other.id !== tag.id))}
              >
                <Icon name="trash" size={14} />
              </IconButton>
            </li>
          )
        })}
      </ul>
      <Button className={styles.add} onClick={add}>
        <Icon name="plus" />
        {m.add}
      </Button>
      {/* Always there, so an error never moves the list. */}
      <p className={styles.error} role="alert">
        {error}
      </p>
    </>
  )
}

interface PaletteProps {
  current: TagColor
  label: (color: TagColor) => string
  onPick: (color: TagColor) => void
  onClose: () => void
}

/** The colors a tag can take, under its swatch. */
function Palette({ current, label, onPick, onClose }: PaletteProps) {
  return (
    <div
      role="radiogroup"
      className={styles.palette}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          // The settings window closes on Escape too; only the palette should.
          event.preventDefault()
          event.stopPropagation()
          onClose()
        }
      }}
    >
      {TAG_COLORS.map((color) => (
        <button
          key={color}
          type="button"
          role="radio"
          aria-checked={color === current}
          aria-label={label(color)}
          title={label(color)}
          autoFocus={color === current}
          className={styles.paletteSwatch}
          style={{ background: tagColor(color) }}
          onClick={() => onPick(color)}
        />
      ))}
    </div>
  )
}
