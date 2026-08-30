import { useEffect, useRef, useState } from 'react'
import {
  api,
  type McpConnectionInfo,
  type McpIntegrationStatus,
  type McpStatus,
} from '@/shared/api'

const DOT: Record<McpStatus, string> = {
  disabled: 'bg-faint',
  starting: 'bg-warn animate-pulse',
  listening: 'bg-ok',
  error: 'bg-error',
}

const LABEL: Record<McpStatus, string> = {
  disabled: 'Disabled',
  starting: 'Starting',
  listening: 'Listening',
  error: 'Error',
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function CopyButton({
  label,
  disabled,
  copied,
  onClick,
}: {
  label: string
  disabled: boolean
  copied: boolean
  onClick: () => void
}) {
  const accessibleLabel = copied ? `Copied ${label}` : `Copy ${label}`
  return (
    <button
      type="button"
      title={accessibleLabel}
      aria-label={accessibleLabel}
      disabled={disabled}
      onClick={onClick}
      className={`absolute right-1 top-1 flex size-6 items-center justify-center rounded transition-all duration-200 disabled:opacity-40 ${copied ? 'bg-ok/10 text-ok' : 'text-muted hover:bg-elevated hover:text-fg'}`}
    >
      <span className="relative size-3.5">
        <svg
          viewBox="0 0 16 16"
          fill="none"
          aria-hidden="true"
          className={`absolute inset-0 size-3.5 stroke-current transition-all duration-200 ${copied ? 'scale-75 opacity-0 blur-[2px]' : 'scale-100 opacity-100 blur-none'}`}
        >
          <rect x="5" y="3" width="8" height="10" rx="1" strokeWidth="1.4" />
          <path d="M3 11V4a1 1 0 0 1 1-1" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
        <svg
          viewBox="0 0 16 16"
          fill="none"
          aria-hidden="true"
          className={`absolute inset-0 size-3.5 stroke-current transition-all duration-200 ${copied ? 'scale-100 opacity-100 blur-none' : 'scale-75 opacity-0 blur-[2px]'}`}
        >
          <path d="m3 8.5 3 3L13 4.5" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </span>
    </button>
  )
}

export function McpControl() {
  const root = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const openRef = useRef(false)
  const [info, setInfo] = useState<McpConnectionInfo | null>(null)
  const [busy, setBusy] = useState(false)
  const [requestError, setRequestError] = useState<string | null>(null)
  const [copied, setCopied] = useState<'local' | 'network' | null>(null)
  const [copyError, setCopyError] = useState<string | null>(null)
  const copyTimer = useRef<number | null>(null)
  const copyGeneration = useRef(0)
  const [integrationStatus, setIntegrationStatus] = useState<McpIntegrationStatus | null>(null)
  const [integrationBusy, setIntegrationBusy] = useState<{
    target: 'chatgptCodex' | 'claudeCode'
    action: 'add' | 'remove'
  } | null>(null)
  const [integrationFeedback, setIntegrationFeedback] = useState<{
    ok: boolean
    text: string
  } | null>(null)

  const refresh = async () => {
    try {
      setInfo(await api.mcpConnectionInfo())
      setRequestError(null)
    } catch (error) {
      setRequestError(errorMessage(error))
    }
  }

  const refreshIntegrationStatus = async () => {
    try {
      setIntegrationStatus(await api.mcpIntegrationStatus())
      return true
    } catch (error) {
      if (openRef.current) {
        setIntegrationFeedback({
          ok: false,
          text: `Could not refresh MCP integration status: ${errorMessage(error)}`,
        })
      }
      return false
    }
  }

  useEffect(() => {
    void refresh()
    void refreshIntegrationStatus()
  }, [])

  useEffect(() => {
    if (!open) return
    void refresh()
    const interval = window.setInterval(() => void refresh(), 2000)
    return () => window.clearInterval(interval)
  }, [open])

  useEffect(() => {
    openRef.current = open
    if (open) return
    copyGeneration.current += 1
    if (copyTimer.current !== null) window.clearTimeout(copyTimer.current)
    copyTimer.current = null
    setCopied(null)
    setCopyError(null)
    setIntegrationFeedback(null)
  }, [open])

  useEffect(
    () => () => {
      copyGeneration.current += 1
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current)
    },
    [],
  )

  useEffect(() => {
    if (!open) return
    const close = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false)
    }
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false)
    }
    document.addEventListener('pointerdown', close)
    document.addEventListener('keydown', escape)
    return () => {
      document.removeEventListener('pointerdown', close)
      document.removeEventListener('keydown', escape)
    }
  }, [open])

  const toggle = async (enabled: boolean) => {
    if (busy) return
    setBusy(true)
    setRequestError(null)
    try {
      setInfo(await api.mcpSetEnabled(enabled))
    } catch (error) {
      setRequestError(errorMessage(error))
    } finally {
      setBusy(false)
    }
  }

  const copy = async (target: 'local' | 'network', label: string, text: string) => {
    const generation = ++copyGeneration.current
    if (copyTimer.current !== null) window.clearTimeout(copyTimer.current)
    copyTimer.current = null
    setCopied(null)
    setCopyError(null)
    try {
      await navigator.clipboard.writeText(text)
      if (generation !== copyGeneration.current) return
      setCopied(target)
      copyTimer.current = window.setTimeout(() => {
        if (generation === copyGeneration.current) {
          setCopied(null)
          copyTimer.current = null
        }
      }, 2000)
    } catch {
      if (generation === copyGeneration.current) {
        setCopyError(`Could not copy ${label.toLowerCase()}`)
      }
    }
  }

  const updateIntegration = async (target: 'chatgptCodex' | 'claudeCode') => {
    const state = integrationStatus?.[target]
    if (integrationBusy || !state?.available) return
    const action = state.configured ? 'remove' : 'add'
    setIntegrationBusy({ target, action })
    setIntegrationFeedback(null)
    try {
      const text = await (target === 'chatgptCodex'
        ? action === 'add'
          ? api.mcpAddToChatgptCodex()
          : api.mcpRemoveFromChatgptCodex()
        : action === 'add'
          ? api.mcpAddToClaude()
          : api.mcpRemoveFromClaude())
      if ((await refreshIntegrationStatus()) && openRef.current) {
        setIntegrationFeedback({ ok: true, text })
      }
    } catch (error) {
      if (openRef.current) {
        setIntegrationFeedback({ ok: false, text: errorMessage(error) })
      }
    } finally {
      setIntegrationBusy(null)
    }
  }

  const status: McpStatus = requestError ? 'error' : (info?.status ?? 'starting')
  const shownError = requestError ?? info?.error

  return (
    <div ref={root} className="relative">
      <button
        type="button"
        title={`MCP: ${LABEL[status]}`}
        aria-label={`MCP server: ${LABEL[status]}`}
        aria-expanded={open}
        aria-controls="mcp-status-popover"
        onClick={() => {
          setOpen((value) => !value)
        }}
        className="inline-flex h-5 items-center gap-1.5 rounded px-1.5 text-[11px] text-muted transition-colors hover:bg-elevated hover:text-fg focus-visible:outline focus-visible:outline-live"
      >
        <span aria-hidden="true" className={`size-1.5 rounded-full ${DOT[status]}`} />
        MCP
      </button>

      {open && (
        <section
          id="mcp-status-popover"
          role="dialog"
          aria-label="MCP server settings"
          className="absolute bottom-7 right-0 z-40 w-96 select-none rounded border border-edge bg-elevated p-3 text-left shadow-2xl"
        >
          <div className="mb-3 flex items-center justify-between">
            <span className="text-[12px] font-medium text-fg">
              {info?.mode === 'remoteRepl' ? 'MCP · Remote REPL' : 'MCP server'}
            </span>
            <span className="inline-flex items-center gap-1.5 text-[11px] text-muted">
              <span aria-hidden="true" className={`size-1.5 rounded-full ${DOT[status]}`} />
              {LABEL[status]}
            </span>
          </div>

          {info?.mode === 'remoteRepl' && (
            <div className="mb-3 rounded border border-warn/40 bg-warn/10 p-2">
              <p className="text-[11px] font-medium text-warn">Remote REPL only</p>
              <p className="mt-1 select-text text-[10px] leading-4 text-muted">
                This MCP exposes only <code className="text-fg">wot_exec</code> for Python 2.7 in
                the connected remote game. Client discovery and process control, logs,
                screenshots, mouse, and keyboard are unavailable.
              </p>
              <p className="mt-1 select-text text-[10px] leading-4 text-muted">
                Run the Windows app on the game computer for the full MCP toolset.
              </p>
            </div>
          )}

          <label
            className={`mb-3 flex items-center justify-between text-[11px] text-fg ${busy || !info ? 'cursor-default' : 'cursor-pointer'}`}
          >
            Enabled
            <input
              type="checkbox"
              checked={info?.enabled ?? false}
              disabled={busy || !info}
              onChange={(event) => void toggle(event.target.checked)}
              className="cursor-pointer accent-live disabled:cursor-default"
            />
          </label>

          <div className="space-y-1">
            <span className="text-[10px] font-medium uppercase tracking-wide text-faint">
              Local URL
            </span>
            <div className="relative">
              <code className="block select-text overflow-x-auto rounded bg-panel p-2 pr-9 font-mono text-[10px] text-fg">
                {info?.url ?? 'Loading…'}
              </code>
              <CopyButton
                label="Local URL"
                disabled={!info}
                copied={copied === 'local'}
                onClick={() => info && void copy('local', 'Local URL', info.url)}
              />
            </div>
          </div>

          <div className="mt-3 space-y-1">
            <span className="text-[10px] font-medium uppercase tracking-wide text-faint">
              Network URL
            </span>
            <div className="relative">
              <code className="block select-text overflow-x-auto rounded bg-panel p-2 pr-9 font-mono text-[10px] text-fg">
                {info?.networkUrl ?? 'Loading…'}
              </code>
              <CopyButton
                label="Network URL"
                disabled={!info}
                copied={copied === 'network'}
                onClick={() => info && void copy('network', 'Network URL', info.networkUrl)}
              />
            </div>
          </div>

          {copyError && (
            <span role="alert" className="mt-2 block select-text text-[10px] text-error">
              {copyError}
            </span>
          )}

          {shownError && (
            <p role="alert" className="mt-2 select-text break-words text-[10px] text-error">
              {shownError}
            </p>
          )}

          <div className="mt-3 border-t border-edge pt-3">
            <span className="text-[10px] font-medium uppercase tracking-wide text-faint">
              Register MCP server
            </span>
            <div className="mt-2 flex gap-3">
              <div>
                <button
                  type="button"
                  title={
                    integrationStatus?.chatgptCodex.error ??
                    (integrationStatus?.chatgptCodex.configured
                      ? 'Remove wot_repl from the shared ChatGPT Desktop/Codex config'
                      : 'Add wot_repl to the shared ChatGPT Desktop/Codex config')
                  }
                  disabled={
                    integrationBusy !== null || integrationStatus?.chatgptCodex.available !== true
                  }
                  onClick={() => void updateIntegration('chatgptCodex')}
                  className={`h-6 rounded border px-2 text-[10px] disabled:opacity-40 ${integrationStatus?.chatgptCodex.configured ? 'border-error/40 text-error hover:border-error hover:bg-error/10' : 'border-edge text-muted hover:border-live hover:text-fg'}`}
                >
                  {integrationBusy?.target === 'chatgptCodex'
                    ? integrationBusy.action === 'add'
                      ? 'Adding…'
                      : 'Removing…'
                    : integrationStatus?.chatgptCodex.configured
                      ? 'Remove from ChatGPT / Codex'
                      : 'Add to ChatGPT / Codex'}
                </button>
              </div>
              <div>
                <button
                  type="button"
                  title={
                    integrationStatus?.claudeCode.error ??
                    (integrationStatus?.claudeCode.configured
                      ? 'Remove wot_repl from the user-scoped Claude Code config'
                      : 'Add wot_repl to the user-scoped Claude Code config')
                  }
                  disabled={
                    integrationBusy !== null || integrationStatus?.claudeCode.available !== true
                  }
                  onClick={() => void updateIntegration('claudeCode')}
                  className={`h-6 rounded border px-2 text-[10px] disabled:opacity-40 ${integrationStatus?.claudeCode.configured ? 'border-error/40 text-error hover:border-error hover:bg-error/10' : 'border-edge text-muted hover:border-live hover:text-fg'}`}
                >
                  {integrationBusy?.target === 'claudeCode'
                    ? integrationBusy.action === 'add'
                      ? 'Adding…'
                      : 'Removing…'
                    : integrationStatus?.claudeCode.configured
                      ? 'Remove from Claude'
                      : 'Add to Claude'}
                </button>
              </div>
            </div>
            {integrationStatus?.chatgptCodex.configPath && (
              <p className="mt-2 select-text break-all text-[9px] leading-4 text-faint">
                ChatGPT / Codex config:{' '}
                <code className="text-muted">{integrationStatus.chatgptCodex.configPath}</code>
              </p>
            )}
            {integrationStatus?.chatgptCodex.error && (
              <p role="alert" className="mt-2 select-text break-words text-[10px] text-error">
                {integrationStatus.chatgptCodex.error}
              </p>
            )}
            {integrationStatus?.claudeCode.configPath && (
              <p className="mt-1 select-text break-all text-[9px] leading-4 text-faint">
                Claude Code config:{' '}
                <code className="text-muted">{integrationStatus.claudeCode.configPath}</code>
              </p>
            )}
            {integrationStatus?.claudeCode.error && (
              <p role="alert" className="mt-2 select-text break-words text-[10px] text-error">
                {integrationStatus.claudeCode.error}
              </p>
            )}
            {integrationFeedback && (
              <p
                role={integrationFeedback.ok ? 'status' : 'alert'}
                className={`mt-2 select-text break-words text-[10px] ${integrationFeedback.ok ? 'text-ok' : 'text-error'}`}
              >
                {integrationFeedback.text}
              </p>
            )}
          </div>
        </section>
      )}
    </div>
  )
}
