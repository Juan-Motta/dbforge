import { useEffect, useState } from 'react'
import { getVersion } from '@tauri-apps/api/app'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { ENGINE_LABELS } from '../../domain/connection'
import { LANGUAGES } from '../../lib/i18n/language'
import { useLanguage, useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import type { ThemePreference } from './theme'
import styles from './SettingsDialog.module.css'

type Section = 'appearance' | 'language' | 'about'

const SECTIONS: Section[] = ['appearance', 'language', 'about']

const THEMES: ThemePreference[] = ['dark', 'light', 'system']

interface SettingsDialogProps {
  theme: ThemePreference
  onThemeChange: (theme: ThemePreference) => void
  onClose: () => void
}

/** App preferences. Changes apply as soon as they are picked. */
export function SettingsDialog({ theme, onThemeChange, onClose }: SettingsDialogProps) {
  const m = useMessages(messages)
  const [section, setSection] = useState<Section>('appearance')

  return (
    <Modal title={m.title} onClose={onClose} className={styles.dialog}>
      <div className={styles.body}>
        <nav aria-label={m.sections} className={styles.nav}>
          {SECTIONS.map((id) => (
            <button
              key={id}
              type="button"
              aria-current={section === id ? 'page' : undefined}
              data-autofocus={section === id || undefined}
              className={styles.navItem}
              onClick={() => setSection(id)}
            >
              {m[id]}
            </button>
          ))}
        </nav>
        <main className={styles.content}>
          {section === 'appearance' && <Appearance theme={theme} onThemeChange={onThemeChange} />}
          {section === 'language' && <LanguageSection />}
          {section === 'about' && <About />}
        </main>
      </div>
    </Modal>
  )
}

function Appearance({ theme, onThemeChange }: Omit<SettingsDialogProps, 'onClose'>) {
  const m = useMessages(messages)
  return (
    <>
      <h3 className={styles.heading}>{m.appearance}</h3>
      <p className={styles.intro}>{m.appearanceIntro}</p>
      <div className={`${styles.row} ${styles.stacked}`}>
        <span className={styles.label} id="settings-theme">
          {m.theme}
        </span>
        <div role="radiogroup" aria-labelledby="settings-theme" className={styles.themes}>
          {THEMES.map((value) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={theme === value}
              className={styles.theme}
              onClick={() => onThemeChange(value)}
            >
              <span className={`${styles.preview} ${styles[value]}`} aria-hidden="true">
                <span className={styles.sidebar} />
                <span className={styles.accentLine} />
                <span className={styles.textLine} />
              </span>
              {m[value]}
            </button>
          ))}
        </div>
      </div>
    </>
  )
}

function LanguageSection() {
  const m = useMessages(messages)
  const { language, choose } = useLanguage()
  return (
    <>
      <h3 className={styles.heading}>{m.language}</h3>
      <p className={styles.intro}>{m.languageIntro}</p>
      <div className={styles.row}>
        <div className={styles.rowText}>
          <span className={styles.label} id="settings-language">
            {m.interfaceLanguage}
          </span>
          <span className={styles.help}>{m.interfaceLanguageHelp}</span>
        </div>
        <div role="radiogroup" aria-labelledby="settings-language" className={styles.segmented}>
          {LANGUAGES.map(({ value, label }) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={language === value}
              lang={value}
              onClick={() => choose(value)}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
    </>
  )
}

function About() {
  const m = useMessages(messages)
  const [version, setVersion] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    getVersion()
      .then((value) => current && setVersion(value))
      .catch(() => current && setVersion(''))
    return () => {
      current = false
    }
  }, [])

  return (
    <>
      <h3 className={styles.heading}>{m.about}</h3>
      <div className={styles.product}>
        <span className={styles.mark} aria-hidden="true">
          <Icon name="database" size={22} />
        </span>
        <div>
          <p className={styles.name}>DBForge</p>
          <p className={styles.help}>{m.tagline}</p>
        </div>
      </div>
      <div className={styles.row}>
        <span className={styles.label}>{m.version}</span>
        {/* The space is kept while the version loads, so nothing moves when it arrives. */}
        <span className={styles.value}>{version === null ? ' ' : version || m.versionUnknown}</span>
      </div>
      <div className={styles.row}>
        <span className={styles.label}>{m.engines}</span>
        <span className={styles.engines}>{Object.values(ENGINE_LABELS).join(' · ')}</span>
      </div>
    </>
  )
}
