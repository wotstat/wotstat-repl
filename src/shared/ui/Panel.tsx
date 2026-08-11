import type { ReactNode } from 'react'

interface PanelProps {
  title: string
  actions?: ReactNode
  children: ReactNode
  className?: string
}

export function Panel({ title, actions, children, className = '' }: PanelProps) {
  return (
    <section className={`flex min-h-0 min-w-0 flex-col bg-panel ${className}`}>
      <header className="flex h-8 shrink-0 items-center justify-between border-b border-edge px-3">
        <span className="min-w-0 truncate select-none text-[11px] font-medium tracking-wider text-muted uppercase">
          {title}
        </span>
        {actions}
      </header>
      <div className="min-h-0 flex-1 overflow-hidden">{children}</div>
    </section>
  )
}
