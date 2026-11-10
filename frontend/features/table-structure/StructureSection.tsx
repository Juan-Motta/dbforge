import { Badge, type Tone } from '../../components/Badge'
import { SqlCode } from '../../components/code/SqlCode'
import { Icon } from '../../components/Icon'
import type { SqlDialect } from '../../domain/connection'
import { useMessages } from '../../lib/i18n/useLanguage'
import type { TableStructure } from './api'
import { columnKeys, formatBytes, type KeyKind } from './format'
import { messages } from './messages'
import type { Section } from './sections'
import styles from './StructureSection.module.css'

const KEY_TONES: Record<KeyKind, Tone> = { PK: 'warn', FK: 'info', UNI: 'ok', IDX: 'muted' }

/** One read-only part of a table's structure. */
export function StructureSection({
  structure,
  section,
  dialect,
}: {
  structure: TableStructure
  section: Section
  dialect?: SqlDialect
}) {
  switch (section) {
    case 'columns':
      return <Columns structure={structure} />
    case 'indexes':
      return <Indexes structure={structure} />
    case 'foreignKeys':
      return <ForeignKeys structure={structure} />
    case 'triggers':
      return <Triggers structure={structure} />
    case 'ddl':
      return <SqlCode value={structure.ddl} dialect={dialect} definition />
  }
}

function Columns({ structure }: { structure: TableStructure }) {
  const m = useMessages(messages)
  const keys = columnKeys(structure)
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th className={styles.number}>#</th>
          <th>{m.name}</th>
          <th>{m.type}</th>
          <th>{m.nullable}</th>
          <th>{m.default}</th>
          <th>{m.key}</th>
          <th>{m.comment}</th>
        </tr>
      </thead>
      <tbody>
        {structure.columns.map((column, i) => (
          <tr key={column.name}>
            <td className={styles.number}>{i + 1}</td>
            <td>{column.name}</td>
            <td className={styles.type}>{column.dataType}</td>
            <td aria-label={column.nullable ? m.nullable : m.notNullable}>
              {column.nullable && <Icon name="check" size={13} />}
            </td>
            <td className={column.default === null ? styles.empty : undefined}>{column.default ?? '—'}</td>
            <td>
              <span className={styles.keys}>
                {keys.get(column.name)?.map((kind) => (
                  <Badge key={kind} tone={KEY_TONES[kind]}>
                    {kind}
                  </Badge>
                ))}
              </span>
            </td>
            <td className={styles.prose}>{column.comment}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function Indexes({ structure }: { structure: TableStructure }) {
  const m = useMessages(messages)
  if (structure.indexes.length === 0) return <Empty title={m.noIndexes} />
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th>{m.index}</th>
          <th>{m.method}</th>
          <th>{m.unique}</th>
          <th>{m.columns}</th>
          <th>{m.condition}</th>
          <th className={styles.numeric}>{m.size}</th>
        </tr>
      </thead>
      <tbody>
        {structure.indexes.map((index) => (
          <tr key={index.name}>
            <td>{index.name}</td>
            <td className={styles.type}>{index.method}</td>
            <td className={styles.prose}>{index.primary ? m.primary : index.unique ? m.yes : m.no}</td>
            <td>{index.columns.join(', ')}</td>
            <td className={index.condition === null ? styles.empty : undefined}>{index.condition ?? '—'}</td>
            <td className={styles.numeric}>{formatBytes(index.bytes)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function ForeignKeys({ structure }: { structure: TableStructure }) {
  const m = useMessages(messages)
  if (structure.foreignKeys.length === 0) return <Empty title={m.noForeignKeys} />
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th>{m.constraint}</th>
          <th>{m.columns}</th>
          <th>{m.references}</th>
          <th>{m.onUpdate}</th>
          <th>{m.onDelete}</th>
        </tr>
      </thead>
      <tbody>
        {structure.foreignKeys.map((key) => (
          <tr key={key.name}>
            <td>{key.name}</td>
            <td>{key.columns.join(', ')}</td>
            <td className={styles.reference}>
              {key.referencedTable}({key.referencedColumns.join(', ')})
            </td>
            <td className={styles.prose}>{key.onUpdate}</td>
            <td className={styles.prose}>{key.onDelete}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function Triggers({ structure }: { structure: TableStructure }) {
  const m = useMessages(messages)
  if (structure.triggers.length === 0) return <Empty title={m.noTriggers} detail={m.triggersHelp} />
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th>{m.trigger}</th>
          <th>{m.enabled}</th>
          <th>{m.definition}</th>
        </tr>
      </thead>
      <tbody>
        {structure.triggers.map((trigger) => (
          <tr key={trigger.name}>
            <td>{trigger.name}</td>
            <td className={styles.prose}>{trigger.enabled ? m.yes : m.no}</td>
            <td className={styles.definition}>{trigger.definition}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function Empty({ title, detail }: { title: string; detail?: string }) {
  return (
    <div className={styles.emptyState}>
      <strong>{title}</strong>
      {detail && <p>{detail}</p>}
    </div>
  )
}
