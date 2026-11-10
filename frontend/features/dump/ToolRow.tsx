import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { useMessages } from '../../lib/i18n/useLanguage'
import { chooseToolFile, type DumpTool } from './api'
import { messages } from './messages'
import { useDumpTool } from './useDumpTool'
import styles from './DumpDialogs.module.css'

/**
 * Which pg_dump or psql runs: its version and path, a way to choose another
 * when the one found isn't right, and back to the one found.
 */
export function ToolRow({ name, disabled = false }: { name: DumpTool; disabled?: boolean }) {
  const m = useMessages(messages)
  const tool = useDumpTool(name)
  const choose = async () => {
    const path = await chooseToolFile(tool.state === 'found' ? tool.tool.path : undefined)
    if (path) await tool.change(path)
  }

  return (
    <div className={styles.tool}>
      <Icon name="code" size={14} />
      <div className={styles.toolText}>
        {tool.state === 'found' ? (
          <>
            <span className={styles.toolName}>
              {name} {tool.tool.version}
              {tool.tool.custom && <span className={styles.chosen}>{m.toolChosen}</span>}
            </span>
            {/* Right to left, so a long path shows its end; the path itself stays left to right. */}
            <span className={styles.toolPath} title={tool.tool.path}>
              <bdi>{tool.tool.path}</bdi>
            </span>
          </>
        ) : (
          <span className={tool.state === 'missing' ? styles.error : styles.toolPath}>
            {tool.state === 'missing' ? tool.error : m.toolLooking(name)}
          </span>
        )}
        {tool.refused && <span className={styles.error}>{tool.refused}</span>}
      </div>
      {tool.state === 'found' && tool.tool.custom && (
        <Button variant="ghost" disabled={disabled} onClick={() => void tool.change(null)}>
          {m.toolUseFound}
        </Button>
      )}
      <Button disabled={disabled || tool.state === 'loading'} onClick={() => void choose()}>
        {tool.state === 'missing' ? m.toolChoose(name) : m.change}
      </Button>
    </div>
  )
}
