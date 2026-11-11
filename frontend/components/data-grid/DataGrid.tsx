import { Icon } from '../Icon'
import { isNumericType, shortTypeName } from './columns'
import styles from './DataGrid.module.css'
import { GutterNumber, RowSheet } from './RowGutter'

export interface GridColumn {
  name: string
  /** Server type name, e.g. `integer` or `int4`. */
  dataType: string
  primaryKey?: boolean
}

interface DataGridProps {
  columns: GridColumn[]
  /** Values in text form; `null` is SQL NULL. */
  rows: (string | null)[][]
  /** Row number of the first row, for the left gutter. */
  firstRowNumber?: number
}

/** Read-only grid of rows with a row-number gutter and a sticky header. */
export function DataGrid({ columns, rows, firstRowNumber = 1 }: DataGridProps) {
  const numeric = columns.map((c) => isNumericType(c.dataType))

  const numbers = rows.map((_, r) => <GutterNumber key={r}>{firstRowNumber + r}</GutterNumber>)

  return (
    <RowSheet lastNumber={firstRowNumber + rows.length - 1} numbers={numbers}>
      <table className={styles.grid}>
        <thead>
          <tr>
            {columns.map((column) => (
              <th key={column.name} title={column.dataType}>
                <span className={styles.header}>
                  {column.primaryKey && <Icon name="key" size={13} className={styles.key} />}
                  {column.name}
                  <span className={styles.type}>{shortTypeName(column.dataType)}</span>
                </span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, r) => (
            <tr key={firstRowNumber + r}>
              {row.map((value, c) =>
                value === null ? (
                  <td key={c} className={styles.null}>
                    NULL
                  </td>
                ) : (
                  <td key={c} className={numeric[c] ? styles.numeric : undefined} title={value}>
                    {value}
                  </td>
                ),
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </RowSheet>
  )
}
