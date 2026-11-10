import { useEffect, useState } from 'react'
import { getVersion } from '@tauri-apps/api/app'
import { Callout, CalloutSpace } from '../../components/Callout'
import { Icon, type IconName } from '../../components/Icon'
import { Modal } from '../../components/Modal'
import { Option, OptionList } from '../../components/OptionList'
import { errorMessage } from '../../lib/errors'
import { LANGUAGES } from '../../lib/i18n/language'
import { useLanguage, useMessages } from '../../lib/i18n/useLanguage'
import {
  connectionsApi,
  type CredentialStorage,
  type CredentialStorageInfo,
  type SwitchProgress,
} from '../connections/api'
import { BackupSettings } from '../backup/BackupSettings'
import { ExportSettings } from '../dump/ExportSettings'
import { McpSettings } from '../mcp/McpSettings'
import { TagsSection } from '../tags/TagsSection'
import { messages, type SettingsMessages } from './messages'
import type { ThemePreference } from './theme'
import styles from './SettingsDialog.module.css'

type Section = 'appearance' | 'language' | 'tags' | 'exports' | 'agents' | 'passwords' | 'backup' | 'about'

const SECTIONS: Section[] = [
  'appearance',
  'language',
  'tags',
  'exports',
  'agents',
  'passwords',
  'backup',
  'about',
]

const THEMES: ThemePreference[] = ['dark', 'light', 'system']

interface SettingsDialogProps {
  theme: ThemePreference
  onThemeChange: (theme: ThemePreference) => void
  /** An imported backup changed the connections. */
  onConnectionsChanged: () => void
  onClose: () => void
}

/** App preferences. Changes apply as soon as they are picked. */
export function SettingsDialog({ theme, onThemeChange, onConnectionsChanged, onClose }: SettingsDialogProps) {
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
          {section === 'tags' && <TagsSection />}
          {section === 'exports' && (
            <>
              <h3 className={styles.heading}>{m.exports}</h3>
              <p className={styles.intro}>{m.exportsIntro}</p>
              <ExportSettings />
            </>
          )}
          {section === 'agents' && (
            <>
              <h3 className={styles.heading}>{m.agents}</h3>
              <p className={styles.intro}>{m.agentsIntro}</p>
              <McpSettings />
            </>
          )}
          {section === 'passwords' && <Passwords />}
          {section === 'backup' && (
            <>
              <h3 className={styles.heading}>{m.backup}</h3>
              <p className={styles.intro}>{m.backupIntro}</p>
              <BackupSettings onThemeChange={onThemeChange} onConnectionsChanged={onConnectionsChanged} />
            </>
          )}
          {section === 'about' && <About />}
        </main>
      </div>
    </Modal>
  )
}

function Appearance({ theme, onThemeChange }: Pick<SettingsDialogProps, 'theme' | 'onThemeChange'>) {
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
const STORAGE_ICONS: Record<CredentialStorage, IconName> = { keychain: 'key', file: 'lock' }

function Passwords() {
  const m = useMessages(messages)
  const [info, setInfo] = useState<CredentialStorageInfo | null>(null)
  const [moving, setMoving] = useState(false)
  const [progress, setProgress] = useState<SwitchProgress | null>(null)
  const [outcome, setOutcome] = useState<{ moved: CredentialStorage } | { error: string } | null>(null)

  useEffect(() => {
    let current = true
    connectionsApi
      .credentialStorage()
      .then((value) => current && setInfo(value))
      .catch((failure) => current && setOutcome({ error: errorMessage(failure) }))
    return () => {
      current = false
    }
  }, [])

  const choose = async (storage: CredentialStorage) => {
    if (moving || info?.storage === storage) return
    setMoving(true)
    setProgress(null)
    setOutcome(null)
    const stopListening = await connectionsApi.onSwitchProgress(setProgress)
    try {
      setInfo(await connectionsApi.setCredentialStorage(storage))
      setOutcome({ moved: storage })
    } catch (failure) {
      setOutcome({ error: errorMessage(failure) })
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
      <div className={styles.sections}>
        <section className={styles.section}>
          <h4 className={styles.sectionTitle} id="settings-storage">
            {m.storage}
          </h4>
          <OptionList role="radiogroup" aria-labelledby="settings-storage">
            {STORAGES.map((storage) => (
              <Option
                key={storage}
                type="radio"
                name="settings-storage"
                icon={STORAGE_ICONS[storage]}
                title={m[storage]}
                description={storage === 'file' ? m.fileHelp : m.keychainHelp}
                checked={info?.storage === storage}
                disabled={info === null || moving}
                onChange={() => void choose(storage)}
              />
            ))}
          </OptionList>
        </section>
        <section className={styles.section}>
          <h4 className={styles.sectionTitle}>{m.fileLocation}</h4>
          <div className={styles.fileRow}>
            <Icon name="lock" size={14} />
            {/* The whole path, wrapped where it must, so it can be read and copied. */}
            <span className={styles.location}>{info?.filePath ?? '\u00a0'}</span>
          </div>
        </section>
        {moving ? (
          <Callout
            tone="progress"
            title={progressText(progress, m)}
            // The keychain can stop to ask for each password; say so, or the switch looks stuck.
            detail={info?.storage === 'keychain' ? m.systemMayAsk : undefined}
          />
        ) : outcome && 'error' in outcome ? (
          <Callout tone="failed" title={m.moveFailed} detail={outcome.error} />
        ) : outcome ? (
          <Callout tone="done" title={m.movedTo(m[outcome.moved])} />
        ) : (
          <CalloutSpace />
        )}
      </div>
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
    </>
  )
}
