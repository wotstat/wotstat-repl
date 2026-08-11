import { ConnectionBadge, useSession } from '@/entities/session'
import { useEditorCursor } from '@/entities/editor'
import { McpControl } from '@/features/manage-mcp'

export function StatusBar() {
  const line = useEditorCursor((s) => s.line)
  const column = useEditorCursor((s) => s.column)
  const status = useSession((s) => s.status)
  const bufferDir = useSession((s) => s.bufferDir)
  const agentVersion = useSession((s) => s.agentVersion)
  const agentPid = useSession((s) => s.agentPid)

  return (
    <footer className="flex h-7 shrink-0 select-none items-center justify-between border-t border-edge bg-panel px-3 text-[11px] text-muted">
      <div className="flex items-center gap-3">
        <ConnectionBadge />
        {bufferDir && (
          <span className="max-w-80 truncate text-faint" title={bufferDir}>
            {bufferDir}
          </span>
        )}
        {status === 'connected' && (agentVersion || agentPid != null) && (
          <span className="text-faint">
            agent v{agentVersion ?? '?'} · pid {agentPid ?? '?'}
          </span>
        )}
        <span className="text-faint">jedi: idle</span>
      </div>
      <div className="flex items-center gap-3">
        <McpControl />
        <span className="text-faint">
          Ln {line}, Col {column}
        </span>
      </div>
    </footer>
  )
}
