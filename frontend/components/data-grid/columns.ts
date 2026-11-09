// Long names from table metadata and short ones from query results (int4, float8, …),
// plus the MySQL and SQL Server spellings (int, tinyint, float, smallmoney).
const NUMERIC_TYPES =
  /^(tinyint|smallint|mediumint|int|integer|bigint|numeric|decimal|real|float|double|double precision|money|smallmoney|int2|int4|int8|float4|float8|oid)\b/

/** Numbers read better right-aligned. */
export function isNumericType(dataType: string): boolean {
  return NUMERIC_TYPES.test(dataType)
}

/** Shortens long server type names for the column header, e.g. `character varying(80)` → `varchar(80)`. */
export function shortTypeName(dataType: string): string {
  return dataType
    .replace(/^character varying/, 'varchar')
    .replace(/^timestamp(\(\d\))? with time zone/, 'timestamptz$1')
    .replace(/^timestamp(\(\d\))? without time zone/, 'timestamp$1')
    .replace(/^double precision/, 'float8')
    .replace(/^character\b/, 'char')
}

/** Whether the column holds UUIDs natively (PostgreSQL `uuid`, SQL Server `uniqueidentifier`). */
export function isUuidType(dataType: string): boolean {
  return /^(uuid|uniqueidentifier)$/i.test(dataType.trim())
}
