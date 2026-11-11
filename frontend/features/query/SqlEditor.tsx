import { useEffect, useImperativeHandle, useRef, type Ref } from 'react'
import { basicSetup } from 'codemirror'
import { Compartment, EditorState, Prec } from '@codemirror/state'
import { EditorView, keymap } from '@codemirror/view'
import { sqlLanguage, type SqlNames } from '../../components/code/sqlLanguage'
import { sqlTheme } from '../../components/code/sqlTheme'
import type { SqlDialect } from '../../domain/connection'

export interface SqlEditorHandle {
  /** The selected text, or the whole script when nothing is selected. */
  textToRun: () => string
  /** Replaces the whole script, e.g. with a query from the history. */
  replace: (text: string) => void
}

interface SqlEditorProps {
  initialValue?: string
  dialect?: SqlDialect
  /** The database's schemas, tables and columns, completed besides keywords once they arrive. */
  names?: SqlNames
  /** Called on ⌘/Ctrl+Enter with `textToRun()`. */
  onRun: (sql: string) => void
  ref?: Ref<SqlEditorHandle>
}

/** CodeMirror SQL editor. Uncontrolled: it owns its text; ⌘/Ctrl+Enter runs. */
export function SqlEditor({ initialValue = '', dialect, names, onRun, ref }: SqlEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const initialDoc = useRef(initialValue)
  // A tab's engine never changes; the names it completes come later, and the
  // language is swapped in place for them, keeping the text and the undo history.
  const initialDialect = useRef(dialect)
  const language = useRef(new Compartment())

  useImperativeHandle(
    ref,
    () => ({
      textToRun: () => (view.current ? textToRun(view.current) : ''),
      replace: (text: string) => {
        const editor = view.current
        if (!editor) return
        editor.dispatch({
          changes: { from: 0, to: editor.state.doc.length, insert: text },
          selection: { anchor: text.length },
        })
        editor.focus()
      },
    }),
    [],
  )

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
        extensions: [
          Prec.highest(runKey),
          basicSetup,
          language.current.of(sqlLanguage(initialDialect.current)),
          sqlTheme,
        ],
      }),
    })
    view.current = editor
    editor.focus()
    return () => {
      editor.destroy()
      view.current = null
    }
  }, [])

  useEffect(() => {
    view.current?.dispatch({
      effects: language.current.reconfigure(sqlLanguage(initialDialect.current, names)),
    })
  }, [names])

  return <div ref={host} style={{ height: '100%' }} />
}

function textToRun(view: EditorView): string {
  const { from, to } = view.state.selection.main
  return from === to ? view.state.doc.toString() : view.state.sliceDoc(from, to)
}
