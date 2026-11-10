import styles from './TagBadge.module.css'
import { tagColor, type Tag } from './tags'

/** A tag's name in its color, as connections show it. */
export function TagBadge({ tag }: { tag: Tag }) {
  return (
    <span className={styles.badge} style={{ '--tag': tagColor(tag.color) } as React.CSSProperties}>
      {tag.name}
    </span>
  )
}
