import { describe, expect, test } from 'bun:test'
import { beginSeparatorResize } from './resizeSeparator'

describe('beginSeparatorResize', () => {
  test('prevents native text selection while capturing the drag pointer', () => {
    let capturedPointer: number | undefined
    let defaultPrevented = false

    beginSeparatorResize({
      pointerId: 7,
      currentTarget: {
        setPointerCapture(pointerId) {
          capturedPointer = pointerId
        },
      },
      preventDefault() {
        defaultPrevented = true
      },
    })

    expect(capturedPointer).toBe(7)
    expect(defaultPrevented).toBe(true)
  })
})
