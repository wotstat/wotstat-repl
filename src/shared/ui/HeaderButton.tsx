import type { ButtonHTMLAttributes } from 'react'

export function HeaderButton({ className = '', ...props }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={`h-6 rounded border border-edge px-2 text-[11px] text-fg transition-colors hover:border-live disabled:opacity-40 ${className}`}
      {...props}
    />
  )
}
