import { useState } from 'react'
import { Badge } from '../../components/Badge'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { DataGrid } from '../../components/data-grid/DataGrid'
import type { QueryOutcome, StatementResult } from './api'
import styles from './QueryConsole.module.css'

function summary(result: StatementResult): string {
  if (result.columns.length === 0) return `${result.rowsAffected} rows affected`
  const count = result.rowsAffected.toLocaleString('en')
  return result.truncated ? `${count} rows · showing first ${result.rows.length}` : `${count} rows`
}

interface ResultsPanelProps {
  outcome: QueryOutcome
  onExport: (result: StatementResult) => void
}

/** Results of one run: a tab per statement, showing the last one first. */
export function ResultsPanel({ outcome, onExport }: ResultsPanelProps) {
  const { results, elapsedMs } = outcome
  const [selected, setSelected] = useState(results.length - 1)
  const result = results[selected]

  if (!result) return <p className={styles.placeholder}>The script had no statements.</p>

  return (
    <>
      <div className={styles.resultsBar}>
        {results.length > 1 && (
          <div role="tablist" className={styles.resultTabs}>
            {results.map((_, i) => (
              <button
                key={i}
                type="button"
                role="tab"
                aria-selected={i === selected}
                onClick={() => setSelected(i)}
              >
                Result {i + 1}
              </button>
            ))}
          </div>
        )}
        <Badge tone="ok">{summary(result)}</Badge>
        <span className="muted">{elapsedMs} ms</span>
        {result.columns.length > 0 && (
          <Button variant="ghost" className={styles.export} onClick={() => onExport(result)}>
            <Icon name="download" />
            Export
          </Button>
        )}
      </div>
      <div className={styles.grid}>
        {result.columns.length > 0 ? (
          <DataGrid columns={result.columns} rows={result.rows} />
        ) : (
          <p className={styles.placeholder}>Statement executed.</p>
        )}
      </div>
    </>
  )
}
