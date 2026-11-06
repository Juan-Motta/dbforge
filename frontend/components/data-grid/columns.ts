// Long names from table metadata and short ones from query results (int4, float8, …).
const NUMERIC_TYPES =
  /^(smallint|integer|bigint|numeric|decimal|real|double precision|money|int2|int4|int8|float4|float8|oid)\b/

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
