import { useEffect, useState, type FormEvent } from 'react'
import { Command } from 'cmdk'
import { disconnect } from '@/features/connect-session'
import { consoleBus } from '@/entities/console'
import { loadState, saveState } from '@/shared/lib'
import {
  COMPLETION_BUDGET_STORAGE_KEY,
  DEFAULT_COMPLETION_BUDGET,
  MAX_COMPLETION_BUDGET,
} from '@/shared/config'

type PaletteCommand = {
  id: string
  title: string
  run?: () => void
}

const COMMANDS: PaletteCommand[] = [
  { id: 'clear', title: 'Clear console', run: () => consoleBus.clear() },
  { id: 'disconnect', title: 'Disconnect session', run: () => void disconnect() },
  { id: 'completion-budget', title: 'Set signature budget' },
]

export function CommandPalette() {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [budgetEditing, setBudgetEditing] = useState(false)
  const [budgetValue, setBudgetValue] = useState('')
  const [budgetError, setBudgetError] = useState<string | null>(null)

  const cancelBudgetEdit = () => {
    setBudgetEditing(false)
    setBudgetError(null)
    setQuery('')
  }

  const closePalette = () => {
    cancelBudgetEdit()
    setOpen(false)
  }

  const openBudgetEditor = () => {
    setBudgetValue(String(loadState<number>(COMPLETION_BUDGET_STORAGE_KEY, DEFAULT_COMPLETION_BUDGET)))
    setBudgetError(null)
    setQuery('')
    setBudgetEditing(true)
  }

  const saveBudget = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const raw = budgetValue.trim()
    const budget = Number(raw)
    if (raw === '' || !Number.isInteger(budget) || budget < 0 || budget > MAX_COMPLETION_BUDGET) {
      setBudgetError(`Enter an integer from 0 to ${MAX_COMPLETION_BUDGET}.`)
      return
    }
    saveState(COMPLETION_BUDGET_STORAGE_KEY, budget)
    closePalette()
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Match the PHYSICAL key via e.code, not e.key: on non-Latin layouts
      // (RU) the K key reports e.key === 'л', so an e.key check never fires.
      // Capture phase so Monaco's Ctrl+K chord can't swallow it first.
      if ((e.ctrlKey || e.metaKey) && e.code === 'KeyK') {
        e.preventDefault()
        e.stopPropagation()
        setOpen((v) => !v)
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [])

  useEffect(() => {
    if (!open) cancelBudgetEdit()
  }, [open])

  return (
    <Command.Dialog
      open={open}
      onOpenChange={setOpen}
      label="Command palette"
      overlayClassName="fixed inset-0 z-50 bg-black/40"
      contentClassName="fixed left-1/2 top-32 z-50 w-[480px] -translate-x-1/2 overflow-hidden rounded-lg border border-edge bg-panel shadow-2xl"
    >
      {budgetEditing ? (
        <form onSubmit={saveBudget}>
          <div className="flex items-center gap-2 border-b border-edge bg-elevated px-3 py-2">
            <button
              type="button"
              aria-label="Back to commands"
              onClick={cancelBudgetEdit}
              className="rounded px-1 text-muted hover:bg-panel hover:text-fg"
            >
              ←
            </button>
            <span className="text-[13px] font-medium text-fg">Set signature budget</span>
          </div>
          <div className="space-y-3 p-3">
            <label htmlFor="signature-budget" className="block text-[12px] text-muted">
              Maximum live candidates inspected for signatures
            </label>
            <input
              id="signature-budget"
              autoFocus
              type="text"
              inputMode="numeric"
              value={budgetValue}
              onChange={(event) => {
                setBudgetValue(event.target.value)
                setBudgetError(null)
              }}
              onKeyDown={(event) => {
                if (event.key !== 'Escape') return
                event.preventDefault()
                event.stopPropagation()
                cancelBudgetEdit()
              }}
              className="w-full rounded border border-edge bg-panel px-3 py-2 text-[13px] text-fg outline-none focus:border-live"
            />
            {budgetError ? (
              <p role="alert" className="text-[11px] text-error">{budgetError}</p>
            ) : (
              <p className="text-[11px] text-faint">0 disables live signature inspection.</p>
            )}
            <div className="flex justify-end gap-2">
              <button
                type="button"
                onClick={cancelBudgetEdit}
                className="h-7 rounded border border-edge px-3 text-[12px] text-muted hover:border-live hover:text-fg"
              >
                Cancel
              </button>
              <button
                type="submit"
                className="h-7 rounded border border-live/60 bg-live/10 px-3 text-[12px] text-fg hover:bg-live/20"
              >
                Save
              </button>
            </div>
          </div>
        </form>
      ) : (
        <>
          <Command.Input
            value={query}
            onValueChange={setQuery}
            placeholder="Type a command"
            className="w-full border-b border-edge bg-elevated px-3 py-2 text-[13px] text-fg outline-none placeholder:text-faint"
          />
          <Command.List className="max-h-72 overflow-auto py-1">
            <Command.Empty className="px-3 py-2 text-[12px] text-faint">No commands</Command.Empty>
            {COMMANDS.map((c) => (
              <Command.Item
                key={c.id}
                value={c.title}
                onSelect={() => {
                  if (c.id === 'completion-budget') {
                    openBudgetEditor()
                    return
                  }
                  c.run?.()
                  closePalette()
                }}
                className="mx-1 cursor-pointer rounded px-2 py-1.5 text-[13px] text-fg data-[selected=true]:bg-elevated"
              >
                {c.title}
              </Command.Item>
            ))}
          </Command.List>
        </>
      )}
    </Command.Dialog>
  )
}
