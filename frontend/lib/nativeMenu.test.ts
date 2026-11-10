import { describe, expect, it, vi } from 'vitest'
import { keepsNativeMenu, suppressNativeMenu } from './nativeMenu'

/** An element whose `closest` finds an editable ancestor or not. */
const element = (editable: boolean) =>
  ({ closest: vi.fn(() => (editable ? {} : null)) }) as unknown as Element

describe('native right-click menu', () => {
  it('stays in text fields and editors only', () => {
    expect(keepsNativeMenu(element(true))).toBe(true)
    expect(keepsNativeMenu(element(false))).toBe(false)
    expect(keepsNativeMenu(null)).toBe(false)
    expect(keepsNativeMenu({} as EventTarget)).toBe(false)
  })

  it('is prevented everywhere else', () => {
    let listener: ((event: Event) => void) | undefined
    suppressNativeMenu({
      addEventListener: (_type: string, handler: unknown) => (listener = handler as never),
    } as never)
    const event = (target: EventTarget) => ({ target, preventDefault: vi.fn() }) as unknown as Event
    const onPage = event(element(false))
    listener!(onPage)
    expect(onPage.preventDefault).toHaveBeenCalled()
    const inField = event(element(true))
    listener!(inField)
    expect(inField.preventDefault).not.toHaveBeenCalled()
  })
})
