export type ResizePointerEvent = {
  currentTarget: {
    setPointerCapture(pointerId: number): void
  }
  pointerId: number
  preventDefault(): void
}

export function beginSeparatorResize(event: ResizePointerEvent): void {
  event.preventDefault()
  event.currentTarget.setPointerCapture(event.pointerId)
}
