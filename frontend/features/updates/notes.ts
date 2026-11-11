/** A piece of a line: plain, or bold where the notes wrote `**…**`. */
export interface Run {
  text: string
  bold: boolean
}

export type NotesBlock =
  { kind: 'heading'; text: string } | { kind: 'item'; runs: Run[] } | { kind: 'text'; runs: Run[] }

/**
 * The release notes as blocks to draw. They are the Markdown that
 * scripts/release-notes.sh writes: `## ` headings and `- ` items, with
 * `**scope:**` in bold; anything else is a line of text.
 */
export function readNotes(markdown: string): NotesBlock[] {
  return markdown
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line !== '')
    .map((line): NotesBlock => {
      const heading = /^#{1,6}\s+(.*)$/.exec(line)
      if (heading) return { kind: 'heading', text: heading[1] }
      const item = /^[-*]\s+(.*)$/.exec(line)
      if (item) return { kind: 'item', runs: runsOf(item[1]) }
      return { kind: 'text', runs: runsOf(line) }
    })
}

function runsOf(line: string): Run[] {
  return line
    .split(/(\*\*[^*]+\*\*)/)
    .filter((part) => part !== '')
    .map((part) =>
      part.startsWith('**') && part.endsWith('**')
        ? { text: part.slice(2, -2), bold: true }
        : { text: part, bold: false },
    )
}
