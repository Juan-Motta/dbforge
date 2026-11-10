import { useMemo, useState } from 'react'
import { Icon } from '../../components/Icon'
import { useMessages } from '../../lib/i18n/useLanguage'
import { messages } from './messages'
import { formatMs, parsePlan, type PlanNode } from './plan'
import styles from './PlanView.module.css'

/**
 * A statement's plan, from the JSON the server returned: totals on top, the node
 * tree, and the selected node's fields.
 */
export function PlanView({ plan: json }: { plan: unknown }) {
  const m = useMessages(messages)
  const plan = useMemo(() => parsePlan(json, m), [json, m])
  // Start on the costliest node: that is usually what the user came for.
  const [selectedId, setSelectedId] = useState(() => costliest(plan.nodes).id)
  const selected = plan.nodes.find((n) => n.id === selectedId) ?? plan.nodes[0]
  const warnings = plan.nodes.reduce((sum, n) => sum + n.warnings.length, 0)

  return (
    <div className={styles.plan}>
      <div className={styles.stats}>
        {plan.analyzed ? (
          <>
            {plan.executionMs !== null && <Stat label={m.execution} value={formatMs(plan.executionMs)} />}
            {/* MySQL doesn't report planning time. */}
            {plan.planningMs !== null && <Stat label={m.planning} value={formatMs(plan.planningMs)} />}
            <Stat label={m.rowsReturned} value={(plan.rows ?? 0).toLocaleString('en-US')} />
          </>
        ) : (
          <Stat label={m.estimateOnly} value={m.turnOnAnalyze} />
        )}
        <Stat
          label={m.warnings}
          value={warnings === 0 ? m.none : String(warnings)}
          tone={warnings ? 'warn' : undefined}
        />
      </div>

      <div className={styles.body}>
        <div className={styles.tree} role="tree" aria-label={m.planNodes}>
          <div className={`${styles.row} ${styles.header}`} aria-hidden="true">
            <span>{m.node}</span>
            <span className={styles.numeric}>{plan.analyzed ? m.rowsColumn : ''}</span>
            <span className={styles.numeric}>{m.estimatedRows}</span>
            <span className={styles.numeric}>{plan.analyzed ? m.selfTime : m.selfCost}</span>
            <span>{m.share}</span>
          </div>
          {plan.nodes.map((node) => (
            <div
              key={node.id}
              role="treeitem"
              aria-level={node.depth + 1}
              aria-selected={node.id === selected.id}
              tabIndex={0}
              className={styles.row}
              onClick={() => setSelectedId(node.id)}
              onKeyDown={(event) => event.key === 'Enter' && setSelectedId(node.id)}
            >
              <span className={styles.name} style={{ paddingLeft: node.depth * 18 }}>
                <span className={styles.type}>{node.nodeType}</span>
                <span className={styles.detail}>{node.detail}</span>
                {node.warnings.length > 0 && (
                  <Icon name="warning" size={13} className={styles.warnIcon} aria-label={m.hasWarnings} />
                )}
              </span>
              <span className={styles.numeric}>{node.actualRows?.toLocaleString('en-US') ?? ''}</span>
              <span className={styles.numeric}>{node.estimatedRows?.toLocaleString('en-US') ?? '—'}</span>
              <span className={styles.numeric}>
                {node.selfMs !== null ? formatMs(node.selfMs) : node.selfCost.toFixed(1)}
              </span>
              <span className={styles.share}>
                <span className={styles.bar}>
                  <span style={{ width: `${node.share}%` }} data-level={level(node.share)} />
                </span>
                <span className={styles.percent}>{node.share}%</span>
              </span>
            </div>
          ))}
        </div>

        <aside className={styles.details} aria-label={m.nodeDetails}>
          <h3>{selected.nodeType}</h3>
          {selected.warnings.map((warning) => (
            <p key={warning} className={styles.warning}>
              {warning}
            </p>
          ))}
          <dl>
            {selected.fields.map(([key, value]) => (
              <div key={key}>
                <dt>{key}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
        </aside>
      </div>
    </div>
  )
}

function Stat({ label, value, tone }: { label: string; value: string; tone?: 'warn' }) {
  return (
    <div className={styles.stat} data-tone={tone}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  )
}

function costliest(nodes: PlanNode[]): PlanNode {
  return nodes.reduce((best, n) => (n.share > best.share ? n : best), nodes[0])
}

/** Bar color: green for small shares, amber for notable ones, red for the bulk. */
function level(share: number): 'low' | 'mid' | 'high' {
  return share > 40 ? 'high' : share > 10 ? 'mid' : 'low'
}
