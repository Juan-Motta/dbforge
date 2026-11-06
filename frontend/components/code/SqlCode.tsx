import { useEffect, useRef } from 'react'
import { sql, PostgreSQL } from '@codemirror/lang-sql'
import { EditorState } from '@codemirror/state'
import { EditorView, lineNumbers } from '@codemirror/view'
import { sqlTheme } from './sqlTheme'

/** Read-only, highlighted SQL that can still be selected and copied. */
export function SqlCode({ value }: { value: string }) {
  const host = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const view = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: value,
        extensions: [lineNumbers(), sql({ dialect: PostgreSQL }), sqlTheme, EditorState.readOnly.of(true)],
      }),
    })
    return () => view.destroy()
  }, [value])

  return <div ref={host} style={{ height: '100%' }} />
}
