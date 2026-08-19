import type { OutFrame, ServerEvent } from '@/shared/api/dto'

export interface ReplConnection {
  endpoint: string
  waitingForAgent: boolean
}

export interface ReplRuntime {
  connect: (onEvent: (event: ServerEvent) => void) => Promise<ReplConnection>
  disconnect: () => Promise<void>
  execCode: (code: string) => Promise<OutFrame>
  complete: (prefix: string, budget?: number) => Promise<OutFrame>
  inspect: (expr: string) => Promise<OutFrame>
  lintCode: (code: string) => Promise<OutFrame>
}

let configuredRuntime: ReplRuntime | null = null

export function configureReplRuntime(runtime: ReplRuntime): void {
  configuredRuntime = runtime
}

function runtime(): ReplRuntime {
  if (!configuredRuntime) throw new Error('REPL runtime is not configured')
  return configuredRuntime
}

// Stable interface used by features. The application entry point supplies the
// environment-specific adapter before React renders.
export const repl: ReplRuntime = {
  connect: (onEvent) => runtime().connect(onEvent),
  disconnect: () => runtime().disconnect(),
  execCode: (code) => runtime().execCode(code),
  complete: (prefix, budget) => runtime().complete(prefix, budget),
  inspect: (expr) => runtime().inspect(expr),
  lintCode: (code) => runtime().lintCode(code),
}
