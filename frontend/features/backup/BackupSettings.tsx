import { useState } from 'react'
import { Button } from '../../components/Button'
import { Callout, CalloutSpace } from '../../components/Callout'
import { Icon } from '../../components/Icon'
import { Option, OptionList } from '../../components/OptionList'
import { errorMessage } from '../../lib/errors'
import { useLanguage, useMessages } from '../../lib/i18n/useLanguage'
import { useTags } from '../tags/useTags'
import type { ThemePreference } from '../settings/theme'
import {
  backupApi,
  chooseBackupFile,
  chooseBackupTarget,
  type BackupPreview,
  type Imported,
  type ImportParts,
} from './api'
import { messages } from './messages'
import { backupFileName, currentPreferences, knownPreferences, savePreferences } from './preferences'
import styles from './Backup.module.css'

/** How the last export or import went, as data: it reads in the language shown now, which an import may change. */
type Outcome =
  | { kind: 'exported'; connections: number; passwords: number; path: string }
  | { kind: 'imported'; done: Imported }
  | { kind: 'failed'; error: string }
  | null

interface BackupSettingsProps {
  onThemeChange: (theme: ThemePreference) => void
  /** Connections changed: the explorer reads them again. */
  onConnectionsChanged: () => void
}

/**
 * Settings › Backup: write DBForge's configuration to a file, or read one
 * back. Importing shows what the file holds and applies the parts ticked,
 * merging: nothing here is deleted.
 */
export function BackupSettings({ onThemeChange, onConnectionsChanged }: BackupSettingsProps) {
  const m = useMessages(messages)
  const language = useLanguage()
  const tags = useTags()
  const [secrets, setSecrets] = useState(true)
  const [busy, setBusy] = useState(false)
  const [outcome, setOutcome] = useState<Outcome>(null)
  // A file picked to import, what it holds, and the parts to apply.
  const [picked, setPicked] = useState<{ path: string; preview: BackupPreview } | null>(null)
  const [parts, setParts] = useState<ImportParts & { preferences: boolean }>({
    preferences: true,
    connections: true,
    mcp: true,
    toolPaths: true,
  })

  const run = async (work: () => Promise<void>) => {
    setBusy(true)
    setOutcome(null)
    try {
      await work()
    } catch (failure) {
      setOutcome({ kind: 'failed', error: errorMessage(failure) })
    } finally {
      setBusy(false)
    }
  }

  const exportBackup = () =>
    run(async () => {
      const path = await chooseBackupTarget(backupFileName(new Date()))
      if (!path) return
      const done = await backupApi.export(path, currentPreferences(), secrets)
      setOutcome({ kind: 'exported', ...done, path })
    })

  const pick = () =>
    run(async () => {
      const path = await chooseBackupFile()
      if (path) setPicked({ path, preview: await backupApi.read(path) })
    })

  const importBackup = () =>
    run(async () => {
      if (!picked) return
      const done = await backupApi.import(picked.path, parts)
      if (parts.preferences) {
        const applied = savePreferences(picked.preview.preferences)
        if (applied.theme) onThemeChange(applied.theme)
        if (applied.language) language.choose(applied.language)
      }
      if (parts.connections) {
        onConnectionsChanged()
        await tags.reload()
      }
      setPicked(null)
      setOutcome({ kind: 'imported', done })
    })

  const preview = picked?.preview
  const known = preview ? knownPreferences(preview.preferences) : null

  return (
    <div className={styles.backup}>
      <section className={styles.section}>
        <h4 className={styles.sectionTitle}>{m.exportTitle}</h4>
        <OptionList>
          <Option
            type="checkbox"
            icon="key"
            title={m.includeSecrets}
            description={m.includeSecretsDescription}
            checked={secrets}
            disabled={busy}
            onChange={(event) => setSecrets(event.target.checked)}
          />
        </OptionList>
        <div>
          <Button disabled={busy} onClick={() => void exportBackup()}>
            <Icon name="download" />
            {m.exportButton}
          </Button>
        </div>
      </section>

      <section className={styles.section}>
        <h4 className={styles.sectionTitle}>{m.importTitle}</h4>
        <p className={styles.note}>{m.importHelp}</p>
        {preview && known ? (
          <>
            <p className={styles.file}>
              <Icon name="folder" size={14} />
              <span>
                {m.fromFile(picked.path, new Date(preview.exportedAt).toLocaleString(), preview.appVersion)}
              </span>
            </p>
            <OptionList>
              <Option
                type="checkbox"
                icon="settings"
                title={m.partPreferences}
                description={m.partPreferencesDetail(
                  known.theme ?? '—',
                  known.language ?? '—',
                  known.exportFolder,
                )}
                checked={parts.preferences}
                onChange={(event) => setParts({ ...parts, preferences: event.target.checked })}
              />
              <Option
                type="checkbox"
                icon="database"
                title={m.partConnections(preview.newConnections.length, preview.updatedConnections.length)}
                description={m.partConnectionsDetail(preview.passwords, preview.tags)}
                checked={parts.connections}
                disabled={preview.newConnections.length + preview.updatedConnections.length === 0}
                onChange={(event) => setParts({ ...parts, connections: event.target.checked })}
              />
              <Option
                type="checkbox"
                icon="code"
                title={m.partMcp}
                description={
                  preview.mcp
                    ? m.partMcpDetail(preview.mcp.enabled, preview.mcp.port, preview.mcpToken)
                    : m.notInFile
                }
                checked={parts.mcp && preview.mcp !== null}
                disabled={preview.mcp === null}
                onChange={(event) => setParts({ ...parts, mcp: event.target.checked })}
              />
              <Option
                type="checkbox"
                icon="play"
                title={m.partTools}
                description={preview.toolPaths > 0 ? m.partToolsDetail(preview.toolPaths) : m.notInFile}
                checked={parts.toolPaths && preview.toolPaths > 0}
                disabled={preview.toolPaths === 0}
                onChange={(event) => setParts({ ...parts, toolPaths: event.target.checked })}
              />
            </OptionList>
            <div className={styles.actions}>
              <Button variant="ghost" disabled={busy} onClick={() => setPicked(null)}>
                {m.cancel}
              </Button>
              <Button variant="primary" disabled={busy} onClick={() => void importBackup()}>
                <Icon name="upload" />
                {m.importButton}
              </Button>
            </div>
          </>
        ) : (
          <div>
            <Button disabled={busy} onClick={() => void pick()}>
              <Icon name="upload" />
              {m.chooseFile}
            </Button>
          </div>
        )}
      </section>

      <OutcomeCallout outcome={outcome} />
    </div>
  )
}

function OutcomeCallout({ outcome }: { outcome: Outcome }) {
  const m = useMessages(messages)
  if (!outcome) return <CalloutSpace />
  switch (outcome.kind) {
    case 'exported':
      return (
        <Callout
          tone="done"
          title={m.exported(outcome.connections, outcome.passwords)}
          detail={outcome.path}
        />
      )
    case 'failed':
      return <Callout tone="failed" title={m.failed} detail={outcome.error} />
    case 'imported': {
      const { done } = outcome
      const details = [
        done.withoutPassword.length > 0 ? m.needPassword(done.withoutPassword.join(', ')) : null,
        done.missingTools.length > 0 ? m.missingTools(done.missingTools.join(', ')) : null,
      ].filter(Boolean)
      return (
        <Callout
          tone="done"
          title={m.imported(done.added, done.updated)}
          detail={details.join(' ') || undefined}
        />
      )
    }
  }
}
