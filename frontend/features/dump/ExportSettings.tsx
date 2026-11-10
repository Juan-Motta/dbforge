import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { useMessages } from '../../lib/i18n/useLanguage'
import { chooseFolder } from './api'
import { messages } from './messages'
import { ExportFolderUsage } from './ExportFolderUsage'
import { ToolRow } from './ToolRow'
import { useExportFolder } from './useExportFolder'
import styles from './DumpDialogs.module.css'

/** The settings' export section: where exports go, and which pg_dump and psql run. */
export function ExportSettings() {
  const m = useMessages(messages)
  const exportFolder = useExportFolder()
  const pick = async () => {
    const folder = await chooseFolder(exportFolder.folder ?? undefined)
    if (folder) exportFolder.choose(folder)
  }

  return (
    <div className={styles.settings}>
      <section className={styles.section}>
        <h3 className={styles.sectionTitle}>{m.defaultFolder}</h3>
        <div className={styles.folderRow}>
          <Icon name="folder" size={15} />
          <span className={styles.path} title={exportFolder.folder ?? undefined}>
            <bdi>{exportFolder.folder ?? m.lookingForFolder}</bdi>
          </span>
          {exportFolder.custom && (
            <Button variant="ghost" onClick={() => exportFolder.choose(null)}>
              {m.useDefaultFolder}
            </Button>
          )}
          <Button onClick={() => void pick()}>{m.change}</Button>
        </div>
        <p className={styles.note}>{m.defaultFolderHelp}</p>
        {exportFolder.folder && <ExportFolderUsage key={exportFolder.folder} folder={exportFolder.folder} />}
        <p className={styles.note}>{m.exportFilesHelp}</p>
      </section>
      <section className={styles.section}>
        <h3 className={styles.sectionTitle}>{m.clientTools}</h3>
        <ToolRow name="pg_dump" />
        <ToolRow name="psql" />
        <p className={styles.note}>{m.clientToolsHelp}</p>
      </section>
    </div>
  )
}
