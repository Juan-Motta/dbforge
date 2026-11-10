import { useState } from 'react'
import { Badge } from '../../components/Badge'
import { Button } from '../../components/Button'
import { Icon } from '../../components/Icon'
import { DataGrid } from '../../components/data-grid/DataGrid'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { QueryOutcome, StatementResult } from './api'
import { messages } from './messages'
import styles from './QueryConsole.module.css'

function summary(result: StatementResult, m: typeof messages.en): string {
  if (result.columns.length === 0) return m.rowsAffected(result.rowsAffected)
  const count = result.rowsAffected
  return result.truncated ? m.rowsShowingFirst(count, result.rows.length) : m.rows(count)
}

interface ResultsPanelProps {
  outcome: QueryOutcome
  onExport: (result: StatementResult) => void
}

/**
 * Results of one run: a tab per statement, showing the last one first. A
 * script that stopped partway shows its error last, and first, after the
 * statements that ran before it.
 */
export function ResultsPanel({ outcome, onExport }: ResultsPanelProps) {
  const m = useMessages(messages)
  const { results, elapsedMs, error } = outcome
  // The error's tab comes after the results.
  const tabs = results.length + (error ? 1 : 0)
  const [selected, setSelected] = useState(tabs - 1)
  const result = results[selected]
  const showingError = error !== null && selected === results.length

  if (!result && !showingError) return <p className={styles.placeholder}>{m.noStatements}</p>

  return (
    <>
      <div className={styles.resultsBar}>
        {tabs > 1 && (
          <div role="tablist" className={styles.resultTabs}>
            {results.map((_, i) => (
              <button
                key={i}
                type="button"
                role="tab"
                aria-selected={i === selected}
                onClick={() => setSelected(i)}
              >
                {m.result(i + 1)}
              </button>
            ))}
            {error !== null && (
              <button
                type="button"
                role="tab"
                className={styles.errorTab}
                aria-selected={showingError}
                onClick={() => setSelected(results.length)}
              >
                {m.stoppedTab}
              </button>
            )}
          </div>
        )}
        {result && <Badge tone="ok">{summary(result, m)}</Badge>}
        <span className="muted">{elapsedMs} ms</span>
        {result && result.columns.length > 0 && (
          <Button variant="ghost" className={styles.export} onClick={() => onExport(result)}>
            <Icon name="download" />
            {m.export}
          </Button>
        )}
      </div>
      <div className={styles.grid}>
        {showingError ? (
          <p className={styles.error}>
            {m.stoppedAfter(results.length)}
            {'\n'}
            {error}
          </p>
        ) : result.columns.length > 0 ? (
          <DataGrid columns={result.columns} rows={result.rows} />
        ) : (
          <p className={styles.placeholder}>{m.statementExecuted}</p>
        )}
      </div>
    </>
  )
}
