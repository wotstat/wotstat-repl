import { ConnectControls } from '@/features/connect-session'
import { disconnect } from '@/features/connect-session'
import { AgentNetworkControl } from '@/features/connect-session'
import { McpControl } from '@/features/manage-mcp'
import { appIconUrl } from '@/shared/assets'
import { StatusBar } from '@/widgets/status-bar'
import { CommandPalette } from '@/widgets/command-palette'
import { ReplWorkspace } from './ReplWorkspace'

export function StudioPage() {
  return (
    <>
      <header className="flex h-10 shrink-0 select-none items-center justify-between border-b border-edge bg-panel px-3">
        <div className="flex items-center gap-2">
          <img src={appIconUrl} alt="" aria-hidden="true" className="size-5 shrink-0" />
          <span className="text-[13px] font-semibold tracking-tight text-fg">WotStat REPL</span>
          <span className="text-[11px] text-faint">Ctrl/Cmd+K for commands</span>
        </div>
        <ConnectControls />
      </header>
      <ReplWorkspace />
      <StatusBar controls={<><AgentNetworkControl /><McpControl /></>} />
      <CommandPalette onDisconnect={() => void disconnect()} />
    </>
  )
}
