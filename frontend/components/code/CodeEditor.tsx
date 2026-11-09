import { useEffect, useRef } from 'react'
import { basicSetup } from 'codemirror'
import { javascript } from '@codemirror/lang-javascript'
import { json } from '@codemirror/lang-json'
import { EditorState } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { sqlTheme } from './sqlTheme'

interface CodeEditorProps {
  value: string
  onChange: (value: string) => void
  /** Highlights JSON, or JavaScript (mongosh's documents); plain text otherwise. */
  language: 'json' | 'javascript' | 'text'
  readOnly?: boolean
  /** Long lines wrap instead of scrolling sideways. */
  wrap?: boolean
  /** Takes the keyboard when it opens, with the cursor at `cursor` (the start by default). */
  autoFocus?: boolean
  cursor?: number
  label: string
}

/** Text with line numbers, folding and the usual editing keys, following the app's theme. */
export function CodeEditor({
  value,
  onChange,
  language,
  readOnly = false,
  wrap = false,
  autoFocus = false,
  cursor = 0,
  label,
}: CodeEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  // The latest text and callback, for an editor rebuilt with other options.
  const latest = useRef({ value, onChange })
  useEffect(() => {
    latest.current = { value, onChange }
  })

  useEffect(() => {
    const editor = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: latest.current.value,
        extensions: [
          basicSetup,
          sqlTheme,
          language === 'json' ? json() : language === 'javascript' ? javascript() : [],
          wrap ? EditorView.lineWrapping : [],
          EditorState.readOnly.of(readOnly),
          EditorView.contentAttributes.of({ 'aria-label': label }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) latest.current.onChange(update.state.doc.toString())
          }),
        ],
      }),
    })
    view.current = editor
    if (autoFocus) {
      editor.dispatch({ selection: { anchor: Math.min(cursor, editor.state.doc.length) } })
      editor.focus()
    }
    return () => {
      editor.destroy()
      view.current = null
    }
  }, [language, readOnly, wrap, autoFocus, cursor, label])

  // Text set from outside (a reformat, a revert) replaces what the editor holds.
  useEffect(() => {
    const editor = view.current
    if (editor && editor.state.doc.toString() !== value) {
      editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: value } })
    }
  }, [value])

  return <div ref={host} style={{ height: '100%', minHeight: 0, overflow: 'hidden' }} />
}
