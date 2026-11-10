import { useEffect, useState } from 'react'
import { getVersion } from '@tauri-apps/api/app'
import { Icon } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { ENGINE_LABELS } from '../../domain/connection'
import { errorMessage } from '../../lib/errors'
import { LANGUAGES } from '../../lib/i18n/language'
import { useLanguage, useMessages } from '../../lib/i18n/useLanguage'
import {
  connectionsApi,
  type CredentialStorage,
  type CredentialStorageInfo,
  type SwitchProgress,
} from '../connections/api'
import { messages, type SettingsMessages } from './messages'
import type { ThemePreference } from './theme'
import styles from './SettingsDialog.module.css'

type Section = 'appearance' | 'language' | 'passwords' | 'about'

const SECTIONS: Section[] = ['appearance', 'language', 'passwords', 'about']

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
          {section === 'passwords' && <Passwords />}
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

const STORAGES: CredentialStorage[] = ['keychain', 'file']

function Passwords() {
  const m = useMessages(messages)
  const [info, setInfo] = useState<CredentialStorageInfo | null>(null)
  const [moving, setMoving] = useState(false)
  const [progress, setProgress] = useState<SwitchProgress | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    connectionsApi
      .credentialStorage()
      .then((value) => current && setInfo(value))
      .catch((failure) => current && setError(errorMessage(failure)))
    return () => {
      current = false
    }
  }, [])

  const choose = async (storage: CredentialStorage) => {
    if (moving || info?.storage === storage) return
    setMoving(true)
    setProgress(null)
    setError(null)
    const stopListening = await connectionsApi.onSwitchProgress(setProgress)
    try {
      setInfo(await connectionsApi.setCredentialStorage(storage))
    } catch (failure) {
      setError(errorMessage(failure))
      // A partly finished switch may still have changed the choice.
      setInfo(await connectionsApi.credentialStorage().catch(() => info))
    } finally {
      stopListening()
      setMoving(false)
    }
  }

  return (
    <>
      <h3 className={styles.heading}>{m.passwords}</h3>
      <p className={styles.intro}>{m.passwordsIntro}</p>
      <div className={`${styles.row} ${styles.stacked}`}>
        <span className={styles.label} id="settings-storage">
          {m.storage}
        </span>
        {/* Each choice shows what it protects against, side by side. */}
        <div role="radiogroup" aria-labelledby="settings-storage" className={styles.choices}>
          {STORAGES.map((storage) => (
            <button
              key={storage}
              type="button"
              role="radio"
              aria-checked={info?.storage === storage}
              disabled={info === null || moving}
              className={styles.choice}
              onClick={() => void choose(storage)}
            >
              <strong>{m[storage]}</strong>
              <span className={styles.help}>{storage === 'file' ? m.fileHelp : m.keychainHelp}</span>
            </button>
          ))}
        </div>
      </div>
      <div className={styles.row}>
        <span className={styles.label}>{m.fileLocation}</span>
        <span className={`${styles.value} ${styles.path}`} title={info?.filePath}>
          {/* Right to left, so a long path shows its end; the path itself stays left to right. */}
          <bdi>{info?.filePath ?? '\u00a0'}</bdi>
        </span>
      </div>
      <p className={styles.status} role="status">
        {moving ? (
          <>
            {progressText(progress, m)}
            {/* The keychain can stop to ask for each password; say so, or the switch looks stuck. */}
            {info?.storage === 'keychain' && <span className={styles.help}> {m.systemMayAsk}</span>}
          </>
        ) : (
          error && <span className={styles.error}>{error}</span>
        )}
      </p>
    </>
  )
}

function progressText(progress: SwitchProgress | null, m: SettingsMessages): string {
  if (progress === null) return m.moving
  const write = progress.phase === 'moving' ? m.movingOne : m.removingOne
  return write(progress.current, progress.total)
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
