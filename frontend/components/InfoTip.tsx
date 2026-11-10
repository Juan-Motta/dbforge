import { Icon } from './Icon'
import styles from './InfoTip.module.css'

/** A small "i" next to a label; its text shows on hover and is read with the field. */
export function InfoTip({ text }: { text: string }) {
  return (
    <span className={styles.info} title={text} aria-label={text} role="img">
      <Icon name="info" size={12} />
    </span>
  )
}
