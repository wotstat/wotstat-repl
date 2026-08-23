import type { ReactNode } from 'react'
import { appIconUrl } from '@/shared/assets'
import { APP_NAME, APP_VERSION } from '@/shared/config'

export function AppHeader({ actions }: { actions?: ReactNode }) {
  return (
    <header className="flex h-10 shrink-0 select-none items-center justify-between border-b border-edge bg-panel px-3">
      <div className="flex items-center gap-2">
        <img src={appIconUrl} alt="" aria-hidden="true" className="size-5 shrink-0" />
        <span className="text-[13px] font-semibold tracking-tight text-fg">{APP_NAME}</span>
        <span className="text-[11px] text-faint">v{APP_VERSION}</span>
        <span className="text-[11px] text-faint">Ctrl/Cmd+K for commands</span>
      </div>
      {actions}
    </header>
  )
}
