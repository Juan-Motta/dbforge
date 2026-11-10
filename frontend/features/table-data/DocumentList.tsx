import { useState, type KeyboardEvent } from 'react'
import { Button } from '../../components/Button'
import { ContextMenu, type MenuItem } from '../../components/ContextMenu'
import { CodeEditor } from '../../components/code/CodeEditor'
import { useMessages } from '../../lib/i18n/useLanguage'
import { documentTokens } from './documentTokens'
import type { Edits, RowChange } from './edits'
import { messages } from './messages'
import styles from './DocumentList.module.css'

type Values = Record<string, string | null>

/** A loaded document: its row's key, the id its selection and edits go by, and its text. */
export interface LoadedDocument {
  id: string
  key: Values
  text: string
}

interface DocumentListProps {
  documents: LoadedDocument[]
  edits: Edits
  /** mongosh's documents are JavaScript literals; Cosmos DB's are JSON. */
  language: 'json' | 'javascript'
  /** The number of the page's first document, as the grid numbers rows. */
  firstNumber: number
  /** Why documents can't be edited now, or `null` when they can. */
  locked: string | null
  selected: Set<string>
  onSelect: (id: string, additive: boolean) => void
  /** The document open in an editor: a loaded one's id, or `doc:<n>` for a new one. */
  editing: string | null
  onEdit: (id: string | null) => void
  /** Checks a change before it is kept; resolves to what is wrong, or `null`. */
  check: (change: RowChange) => Promise<string | null>
  onReplace: (key: Values, text: string, original: string) => void
  onEditNew: (id: number, text: string) => void
  onRemoveNew: (id: number) => void
  /** Marks a loaded document for deletion, or keeps it again. */
  onToggleDelete: (key: Values) => void
}

/** The page's rows as whole documents; a double click edits one, as a cell in the grid. */
export function DocumentList({
  documents,
  edits,
  language,
  firstNumber,
  locked,
  selected,
  onSelect,
  editing,
  onEdit,
  check,
  onReplace,
  onEditNew,
  onRemoveNew,
  onToggleDelete,
}: DocumentListProps) {
  const m = useMessages(messages)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)
  const write = (text: string) => void navigator.clipboard.writeText(text)

  /** The texts of the selected documents as they stand, in the list's order. */
  const selectedTexts = () => [
    ...edits.newDocuments.filter(({ id }) => selected.has(`doc:${id}`)).map(({ document }) => document),
    ...documents
      .filter(({ id }) => selected.has(id))
      .map(({ id, text }) => edits.replacements.get(id)?.document ?? text),
  ]

  /** A document's menu: edit, copy (it, or the selected ones), and delete or keep it. */
  const menuOf = (id: string, text: string, state: 'edited' | 'deleted' | 'new' | undefined): MenuItem[] => {
    const count = selected.size > 1 && selected.has(id) ? selected.size : 0
    const loaded = documents.find((document) => document.id === id)
    return [
      ...(!locked && state !== 'deleted'
        ? [{ label: m.editDocument, icon: 'edit' as const, onSelect: () => onEdit(id) }]
        : []),
      {
        label: m.copyMenu,
        icon: 'duplicate',
        items: [
          { label: m.copyDocument, onSelect: () => write(text) },
          ...(count
            ? [
                {
                  label: m.copySelectedDocuments(count),
                  onSelect: () => write(`[\n${selectedTexts().join(',\n')}\n]`),
                },
              ]
            : []),
        ],
      },
      ...(locked
        ? []
        : state === 'new'
          ? [
              {
                label: m.removeNewDocument,
                icon: 'trash' as const,
                onSelect: () => onRemoveNew(Number(id.slice(4))),
              },
            ]
          : loaded
            ? [
                {
                  label: state === 'deleted' ? m.keepDocument : m.deleteDocument,
                  icon: 'trash' as const,
                  onSelect: () => onToggleDelete(loaded.key),
                },
              ]
            : []),
    ]
  }
  const stateLabels = { edited: m.stateEdited, deleted: m.stateDeleted, new: m.stateNew }
  const card = (props: {
    id: string
    label: string
    text: string
    state?: 'edited' | 'deleted' | 'new'
    save: (text: string) => Promise<string | null>
    cancel: () => void
  }) => {
    const isEditing = editing === props.id
    return (
      <article
        key={props.id}
        className={styles.document}
        data-state={props.state}
        aria-selected={selected.has(props.id)}
        onClick={(event) => !isEditing && onSelect(props.id, event.metaKey || event.ctrlKey)}
        onDoubleClick={() => !isEditing && !locked && props.state !== 'deleted' && onEdit(props.id)}
        onContextMenu={(event) => {
          if (isEditing) return
          event.preventDefault()
          // The menu acts on what was right-clicked, keeping a selection it belongs to.
          if (!selected.has(props.id)) onSelect(props.id, false)
          setMenu({ x: event.clientX, y: event.clientY, items: menuOf(props.id, props.text, props.state) })
        }}
      >
        <header className={styles.header}>
          {props.label && <span className={styles.number}>{props.label}</span>}
          {props.state && <span className={styles.state}>{stateLabels[props.state]}</span>}
        </header>
        {isEditing ? (
          <DocumentEditor
            initial={props.text}
            language={language}
            onSave={props.save}
            onCancel={props.cancel}
          />
        ) : (
          <pre className={styles.code}>
            {documentTokens(props.text).map((token, i) =>
              token.kind === 'plain' ? (
                token.text
              ) : (
                <span key={i} className={styles[token.kind]}>
                  {token.text}
                </span>
              ),
            )}
          </pre>
        )}
      </article>
    )
  }

  return (
    <div className={styles.list} title={locked ?? undefined}>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {edits.newDocuments.map(({ id, document }) =>
        card({
          id: `doc:${id}`,
          label: '',
          text: document,
          state: 'new',
          save: async (text) => {
            const problem = await check({ kind: 'insertDocument', document: text })
            if (problem === null) {
              onEditNew(id, text)
              onEdit(null)
            }
            return problem
          },
          cancel: () => {
            // A new document never saved is no document at all.
            if (document === NEW_DOCUMENT) onRemoveNew(id)
            onEdit(null)
          },
        }),
      )}
      {documents.map(({ id, key, text }, index) => {
        const replaced = edits.replacements.get(id)?.document
        return card({
          id,
          label: `#${firstNumber + index}`,
          text: replaced ?? text,
          state: edits.deletes.has(id) ? 'deleted' : replaced !== undefined ? 'edited' : undefined,
          save: async (next) => {
            const problem = next === text ? null : await check({ kind: 'replace', key, document: next })
            if (problem === null) {
              onReplace(key, next, text)
              onEdit(null)
            }
            return problem
          },
          cancel: () => onEdit(null),
        })
      })}
    </div>
  )
}

/** The text a new document starts with. */
export const NEW_DOCUMENT = '{\n  \n}'

/** Rows the editor shows at most before it scrolls. */
const MAX_LINES = 24
const LINE_HEIGHT = 18

/** A document being edited: ⌘↵ keeps it once the database can read it, Escape drops the edit. */
function DocumentEditor({
  initial,
  language,
  onSave,
  onCancel,
}: {
  initial: string
  language: 'json' | 'javascript'
  onSave: (text: string) => Promise<string | null>
  onCancel: () => void
}) {
  const m = useMessages(messages)
  const [draft, setDraft] = useState(initial)
  const [problem, setProblem] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  // Sized once for the text it opens with, so typing never moves what is below.
  const [height] = useState(() => (Math.min(MAX_LINES, initial.split('\n').length + 2) + 1) * LINE_HEIGHT)

  const save = async () => {
    setSaving(true)
    setProblem(await onSave(draft))
    setSaving(false)
  }

  // Caught on the way down, before the editor takes ⌘↵ for a new line.
  const onKeyDownCapture = (event: KeyboardEvent) => {
    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault()
      event.stopPropagation()
      void save()
    } else if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      onCancel()
    }
  }

  return (
    <div className={styles.editor} onKeyDownCapture={onKeyDownCapture}>
      <div style={{ height }}>
        <CodeEditor
          value={draft}
          onChange={setDraft}
          language={language}
          label={m.document}
          autoFocus
          // A new document opens on its empty line, inside the braces.
          cursor={initial === NEW_DOCUMENT ? NEW_DOCUMENT.indexOf('\n  ') + 3 : 0}
        />
      </div>
      <footer className={styles.editorBar}>
        <span className={styles.problem} title={problem ?? undefined}>
          {problem}
        </span>
        <Button onClick={onCancel}>{m.cancel}</Button>
        <Button variant="primary" disabled={saving} onClick={() => void save()}>
          {m.keep}
          <span className={styles.shortcut}>⌘↵</span>
        </Button>
      </footer>
    </div>
  )
}
