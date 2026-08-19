import { Channel, invoke } from '@tauri-apps/api/core'
import type { AgentConnectionInfo, OutFrame, ServerEvent } from '@/shared/api/dto'
import {
  AGENT_LAN_STORAGE_KEY,
  AGENT_SECURE_STORAGE_KEY,
} from '@/shared/config'
import { loadState } from '@/shared/lib'
import { completionBudget } from './completionBudget'
import type { ReplRuntime } from './runtime'

export const tauriReplRuntime: ReplRuntime = {
  async connect(onEvent) {
    const lanEnabled = loadState(AGENT_LAN_STORAGE_KEY, false)
    const secureEnabled = loadState(AGENT_SECURE_STORAGE_KEY, true)
    const info = await invoke<AgentConnectionInfo>('agent_connection_info')
    const channel = new Channel<ServerEvent>()
    channel.onmessage = onEvent
    await invoke<void>('connect', { lanEnabled, secureEnabled, onEvent: channel })
    return {
      endpoint: lanEnabled ? info.networkAddress : info.localAddress,
      waitingForAgent: true,
    }
  },

  disconnect: () => invoke<void>('disconnect'),
  execCode: (code) => invoke<OutFrame>('exec_code', { code }),
  complete: (prefix, budget = completionBudget()) =>
    invoke<OutFrame>('complete', { prefix, budget }),
  inspect: (expr) => invoke<OutFrame>('inspect', { expr }),
  lintCode: (code) => invoke<OutFrame>('lint_code', { code }),
}
