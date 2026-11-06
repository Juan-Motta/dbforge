import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { EditorView } from '@codemirror/view'
import { tags } from '@lezer/highlight'

// Colors come from the theme tokens, so the editor follows light/dark mode.
const editorTheme = EditorView.theme({
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

/** Editor colors and SQL syntax colors, shared by every SQL view. */
export const sqlTheme = [editorTheme, syntaxHighlighting(highlight)]
