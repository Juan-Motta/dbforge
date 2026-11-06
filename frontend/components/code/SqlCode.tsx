import { useEffect, useRef } from 'react'
import { EditorState } from '@codemirror/state'
import { EditorView, lineNumbers } from '@codemirror/view'
import type { SqlDialect } from '../../domain/connection'
import { sqlLanguage } from './sqlLanguage'
import { sqlTheme } from './sqlTheme'

/** Read-only, highlighted SQL that can still be selected and copied. */
export function SqlCode({ value, dialect }: { value: string; dialect?: SqlDialect }) {
  const host = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const view = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: value,
        extensions: [lineNumbers(), sqlLanguage(dialect), sqlTheme, EditorState.readOnly.of(true)],
      }),
    })
    return () => view.destroy()
  }, [value, dialect])

  return <div ref={host} style={{ height: '100%' }} />
}
