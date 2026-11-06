import { useEffect, useImperativeHandle, useRef, type Ref } from 'react'
import { basicSetup } from 'codemirror'
import { EditorState, Prec } from '@codemirror/state'
import { EditorView, keymap } from '@codemirror/view'
import { sqlLanguage } from '../../components/code/sqlLanguage'
import { sqlTheme } from '../../components/code/sqlTheme'
import type { SqlDialect } from '../../domain/connection'

export interface SqlEditorHandle {
  /** The selected text, or the whole script when nothing is selected. */
  textToRun: () => string
}

interface SqlEditorProps {
  initialValue?: string
  dialect?: SqlDialect
  /** Called on ⌘/Ctrl+Enter with `textToRun()`. */
  onRun: (sql: string) => void
  ref?: Ref<SqlEditorHandle>
}

/** CodeMirror SQL editor. Uncontrolled: it owns its text; ⌘/Ctrl+Enter runs. */
export function SqlEditor({ initialValue = '', dialect, onRun, ref }: SqlEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const initialDoc = useRef(initialValue)
  // A tab's engine never changes, so the language is set once with the editor.
  const initialDialect = useRef(dialect)

  useImperativeHandle(ref, () => ({ textToRun: () => (view.current ? textToRun(view.current) : '') }), [])

  // The editor is created once; this keeps it calling the latest callback.
  const onRunRef = useRef(onRun)
  useEffect(() => {
    onRunRef.current = onRun
  }, [onRun])

  useEffect(() => {
    const runKey = keymap.of([
      {
        key: 'Mod-Enter',
        run: (view) => {
          onRunRef.current(textToRun(view))
          return true
        },
      },
    ])
    const editor = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: initialDoc.current,
        extensions: [Prec.highest(runKey), basicSetup, sqlLanguage(initialDialect.current), sqlTheme],
      }),
    })
    view.current = editor
    editor.focus()
    return () => {
      editor.destroy()
      view.current = null
    }
  }, [])

  return <div ref={host} style={{ height: '100%' }} />
}

function textToRun(view: EditorView): string {
  const { from, to } = view.state.selection.main
  return from === to ? view.state.doc.toString() : view.state.sliceDoc(from, to)
}
