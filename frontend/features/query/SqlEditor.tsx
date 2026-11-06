import { useEffect, useImperativeHandle, useRef, type Ref } from 'react'
import { basicSetup } from 'codemirror'
import { sql, PostgreSQL } from '@codemirror/lang-sql'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { EditorState, Prec } from '@codemirror/state'
import { EditorView, keymap } from '@codemirror/view'
import { tags } from '@lezer/highlight'

// Colors come from the theme tokens, so the editor follows light/dark mode.
const theme = EditorView.theme({
  '&': { height: '100%', background: 'var(--surface)', color: 'var(--text)', fontSize: '13px' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '20px' },
  '.cm-content': { padding: '10px 0', caretColor: 'var(--accent)' },
  '.cm-gutters': { background: 'var(--surface)', color: 'var(--text3)', border: 'none' },
  '.cm-activeLine, .cm-activeLineGutter': { background: 'var(--hover)' },
  '.cm-cursor': { borderLeftColor: 'var(--accent)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
    background: 'var(--accent-soft) !important',
  },
  '.cm-tooltip': { background: 'var(--elev)', border: '1px solid var(--border2)', borderRadius: '8px' },
  '.cm-tooltip-autocomplete ul li[aria-selected]': { background: 'var(--accent-soft)', color: 'var(--text)' },
})

const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: 'var(--kw)', fontWeight: '500' },
  { tag: [tags.string, tags.special(tags.string)], color: 'var(--str)' },
  { tag: [tags.number, tags.bool, tags.null], color: 'var(--num)' },
  { tag: [tags.function(tags.variableName), tags.standard(tags.name)], color: 'var(--fn)' },
  { tag: [tags.lineComment, tags.blockComment], color: 'var(--com)', fontStyle: 'italic' },
  { tag: [tags.operator, tags.punctuation], color: 'var(--text2)' },
])

export interface SqlEditorHandle {
  /** The selected text, or the whole script when nothing is selected. */
  textToRun: () => string
}

interface SqlEditorProps {
  initialValue?: string
  /** Called on ⌘/Ctrl+Enter with `textToRun()`. */
  onRun: (sql: string) => void
  ref?: Ref<SqlEditorHandle>
}

/** CodeMirror SQL editor. Uncontrolled: it owns its text; ⌘/Ctrl+Enter runs. */
export function SqlEditor({ initialValue = '', onRun, ref }: SqlEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const initialDoc = useRef(initialValue)

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
        extensions: [
          Prec.highest(runKey),
          basicSetup,
          sql({ dialect: PostgreSQL, upperCaseKeywords: true }),
          syntaxHighlighting(highlight),
          theme,
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

  return <div ref={host} style={{ height: '100%' }} />
}

function textToRun(view: EditorView): string {
  const { from, to } = view.state.selection.main
  return from === to ? view.state.doc.toString() : view.state.sliceDoc(from, to)
}
