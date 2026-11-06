import { Modal } from '../../components/Modal'
import type { ThemePreference } from './theme'
import styles from './SettingsDialog.module.css'

const THEMES: { value: ThemePreference; label: string }[] = [
  { value: 'dark', label: 'Dark' },
  { value: 'light', label: 'Light' },
  { value: 'system', label: 'Match system' },
]

interface SettingsDialogProps {
  theme: ThemePreference
  onThemeChange: (theme: ThemePreference) => void
  onClose: () => void
}

/** App preferences. Changes apply as soon as they are picked. */
export function SettingsDialog({ theme, onThemeChange, onClose }: SettingsDialogProps) {
  return (
    <Modal title="Settings" onClose={onClose} className={styles.dialog}>
      <section className={styles.section}>
        <h3>Appearance</h3>
        <div role="radiogroup" aria-label="Theme" className={styles.themes}>
          {THEMES.map(({ value, label }) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={theme === value}
              data-autofocus={theme === value || undefined}
              className={styles.theme}
              onClick={() => onThemeChange(value)}
            >
              <span className={`${styles.preview} ${styles[value]}`} aria-hidden="true">
                <span className={styles.sidebar} />
                <span className={styles.accentLine} />
                <span className={styles.textLine} />
              </span>
              {label}
            </button>
          ))}
        </div>
      </section>
    </Modal>
  )
}
