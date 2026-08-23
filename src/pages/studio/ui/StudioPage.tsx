import { ConnectControls } from '@/features/connect-session'
import { disconnect } from '@/features/connect-session'
import { AgentNetworkControl } from '@/features/connect-session'
import { McpControl } from '@/features/manage-mcp'
import { StatusBar } from '@/widgets/status-bar'
import { CommandPalette } from '@/widgets/command-palette'
import { AppHeader } from './AppHeader'
import { ReplWorkspace } from './ReplWorkspace'

export function StudioPage() {
  return (
    <>
      <AppHeader actions={<ConnectControls />} />
      <ReplWorkspace />
      <StatusBar controls={<><AgentNetworkControl /><McpControl /></>} />
      <CommandPalette onDisconnect={() => void disconnect()} />
    </>
  )
}
