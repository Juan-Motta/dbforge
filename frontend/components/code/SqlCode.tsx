import { useEffect, useRef } from 'react'
import { EditorState } from '@codemirror/state'
import { EditorView, lineNumbers } from '@codemirror/view'
import type { SqlDialect } from '../../domain/connection'
import { definitionLanguage, sqlLanguage } from './sqlLanguage'
import { sqlTheme } from './sqlTheme'

/**
 * Read-only, highlighted SQL that can still be selected and copied. A
 * `definition` is a table's definition script, which some engines write in JSON.
 */
export function SqlCode({
  value,
  dialect,
  definition = false,
}: {
  value: string
  dialect?: SqlDialect
  definition?: boolean
}) {
  const host = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const view = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          definition ? definitionLanguage(dialect) : sqlLanguage(dialect),
          sqlTheme,
          EditorState.readOnly.of(true),
        ],
      }),
    })
    return () => view.destroy()
  }, [value, dialect, definition])

  return <div ref={host} style={{ height: '100%' }} />
}
