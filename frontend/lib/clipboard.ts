/**
 * Puts `text` on the clipboard. WebKit only lets a click write to the
 * clipboard while the click is handled, so call this from the handler itself:
 * text still on its way (a promise) is written once it arrives.
 */
export async function copyText(text: string | Promise<string>): Promise<void> {
  const blob = Promise.resolve(text).then((value) => new Blob([value], { type: 'text/plain' }))
  await navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob })])
}
