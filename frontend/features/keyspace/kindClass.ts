import styles from './KeyBrowser.module.css'

/** The badge colors of a key type; module types share a neutral one. */
export function kindClass(kind: string): string {
  return styles[`kind-${kind}`] ?? styles['kind-other']
}
