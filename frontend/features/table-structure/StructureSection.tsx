import { Badge, type Tone } from '../../components/Badge'
import { SqlCode } from '../../components/code/SqlCode'
import { Icon } from '../../components/Icon'
import type { SqlDialect } from '../../domain/connection'
import type { TableStructure } from './api'
import { columnKeys, formatBytes, type KeyKind } from './format'
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
      return <SqlCode value={structure.ddl} dialect={dialect} />
  }
}

function Columns({ structure }: { structure: TableStructure }) {
  const keys = columnKeys(structure)
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th className={styles.number}>#</th>
          <th>Name</th>
          <th>Type</th>
          <th>Nullable</th>
          <th>Default</th>
          <th>Key</th>
          <th>Comment</th>
        </tr>
      </thead>
      <tbody>
        {structure.columns.map((column, i) => (
          <tr key={column.name}>
            <td className={styles.number}>{i + 1}</td>
            <td>{column.name}</td>
            <td className={styles.type}>{column.dataType}</td>
            <td aria-label={column.nullable ? 'Nullable' : 'Not nullable'}>
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
  if (structure.indexes.length === 0) return <Empty title="No indexes on this table" />
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th>Index</th>
          <th>Method</th>
          <th>Unique</th>
          <th>Columns</th>
          <th>Condition</th>
          <th className={styles.numeric}>Size</th>
        </tr>
      </thead>
      <tbody>
        {structure.indexes.map((index) => (
          <tr key={index.name}>
            <td>{index.name}</td>
            <td className={styles.type}>{index.method}</td>
            <td className={styles.prose}>{index.primary ? 'Primary' : index.unique ? 'Yes' : 'No'}</td>
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
  if (structure.foreignKeys.length === 0) return <Empty title="No foreign keys on this table" />
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th>Constraint</th>
          <th>Columns</th>
          <th>References</th>
          <th>On update</th>
          <th>On delete</th>
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
  if (structure.triggers.length === 0) {
    return (
      <Empty
        title="No triggers on this table"
        detail="Triggers run SQL automatically on insert, update or delete."
      />
    )
  }
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th>Trigger</th>
          <th>Enabled</th>
          <th>Definition</th>
        </tr>
      </thead>
      <tbody>
        {structure.triggers.map((trigger) => (
          <tr key={trigger.name}>
            <td>{trigger.name}</td>
            <td className={styles.prose}>{trigger.enabled ? 'Yes' : 'No'}</td>
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
