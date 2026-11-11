import { DEFAULT_OPTIONS, FORMATS, ROWS_PER_INSERT, type ExportFormat, type FormatOptions } from './formats'

const STORAGE_KEY = 'dbforge.export'

/** The format and options of the last export, offered first next time. */
export interface ExportChoice {
  format: ExportFormat
  options: FormatOptions
}

const DEFAULT_CHOICE: ExportChoice = { format: 'csv', options: DEFAULT_OPTIONS }

/** What was kept, with anything missing or not understood back at its default. */
export function readChoice(saved: string | null): ExportChoice {
  let value: Partial<{ format: unknown; options: Partial<Record<keyof FormatOptions, unknown>> }>
  try {
    value = saved ? JSON.parse(saved) : {}
  } catch {
    return DEFAULT_CHOICE
  }
  const options = value?.options ?? {}
  return {
    format: FORMATS.some((f) => f.id === value?.format) ? (value.format as ExportFormat) : 'csv',
    options: {
      header: typeof options.header === 'boolean' ? options.header : DEFAULT_OPTIONS.header,
      nullText: typeof options.nullText === 'string' ? options.nullText : DEFAULT_OPTIONS.nullText,
      jsonStyle: options.jsonStyle === 'lines' ? 'lines' : 'array',
      rowsPerInsert: ROWS_PER_INSERT.includes(options.rowsPerInsert as (typeof ROWS_PER_INSERT)[number])
        ? (options.rowsPerInsert as number)
        : DEFAULT_OPTIONS.rowsPerInsert,
    },
  }
}

export function loadChoice(): ExportChoice {
  try {
    return readChoice(localStorage.getItem(STORAGE_KEY))
  } catch {
    return DEFAULT_CHOICE
  }
}

export function saveChoice(choice: ExportChoice) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(choice))
  } catch {
    // Not kept for next time; this export still used it.
  }
}
