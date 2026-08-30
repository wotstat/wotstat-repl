import { useEffect, useRef, useState } from 'react'
import {
  api,
  type McpActivityEntry,
  type McpActivityStatus,
  type McpConnectionInfo,
  type McpIntegrationStatus,
  type McpStatus,
} from '@/shared/api'
import { hasPrettyActivity } from '../lib/activityPresentation'
import { McpActivityPresentation } from './McpActivityPresentation'

const RECENT_ACTIVITY_MS = 2 * 60 * 1000
const ACTIVITY_LIMIT = 500

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

const ACTIVITY_DOT: Record<McpActivityStatus, string> = {
  pending: 'bg-warn animate-pulse',
  success: 'bg-ok',
  error: 'bg-error',
}

const ACTIVITY_LABEL: Record<McpActivityStatus, string> = {
  pending: 'Running',
  success: 'Succeeded',
  error: 'Failed',
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function CopyButton({
  label,
  disabled,
  copied,
  placement = 'overlay',
  onClick,
}: {
  label: string
  disabled: boolean
  copied: boolean
  placement?: 'overlay' | 'inline'
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
      className={`${placement === 'overlay' ? 'absolute right-1 top-1' : 'shrink-0'} flex size-6 items-center justify-center rounded transition-all duration-200 disabled:opacity-40 ${copied ? 'bg-ok/10 text-ok' : 'text-muted hover:bg-elevated hover:text-fg'}`}
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
          <path
            d="m3 8.5 3 3L13 4.5"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </span>
    </button>
  )
}

function BackIcon() {
  return (
    <svg viewBox="0 0 16 16" fill="none" aria-hidden="true" className="size-3.5 stroke-current">
      <path d="m9.5 3.5-4.5 4.5 4.5 4.5" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  )
}

function ChevronIcon() {
  return (
    <svg viewBox="0 0 16 16" fill="none" aria-hidden="true" className="size-3 stroke-current">
      <path d="m6 3.5 4.5 4.5L6 12.5" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  )
}

function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

function formatDuration(durationMs: number): string {
  if (durationMs < 1_000) return `${durationMs} ms`
  if (durationMs < 60_000) return `${(durationMs / 1_000).toFixed(durationMs < 10_000 ? 1 : 0)} s`
  const minutes = Math.floor(durationMs / 60_000)
  const seconds = Math.floor((durationMs % 60_000) / 1_000)
  return `${minutes}m ${seconds}s`
}

function activityDuration(entry: McpActivityEntry, now: number): string {
  return formatDuration(entry.durationMs ?? Math.max(0, now - entry.startedAtMs))
}

function payloadText(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2) ?? String(value)
  } catch {
    return String(value)
  }
}

function ActivityRow({
  entry,
  now,
  onClick,
}: {
  entry: McpActivityEntry
  now: number
  onClick: () => void
}) {
  return (
    <button
      type="button"
      title={`Open ${entry.command} details`}
      onClick={onClick}
      className="flex w-full items-center gap-2 border-b border-edge px-3 py-2.5 text-left transition-colors last:border-b-0 hover:bg-elevated/70 focus-visible:outline focus-visible:outline-live"
    >
      <span
        aria-label={ACTIVITY_LABEL[entry.status]}
        title={ACTIVITY_LABEL[entry.status]}
        className={`size-2 shrink-0 rounded-full ${ACTIVITY_DOT[entry.status]}`}
      />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-mono text-[10px] text-fg">{entry.command}</span>
        <span className="mt-0.5 block text-[9px] text-faint">
          {formatTime(entry.startedAtMs)} · {activityDuration(entry, now)}
        </span>
      </span>
      <span className="shrink-0 text-faint">
        <ChevronIcon />
      </span>
    </button>
  )
}

function ActivityPreviewRow({ entry, now }: { entry: McpActivityEntry; now: number }) {
  return (
    <div className="flex items-center gap-2 rounded px-2 py-1.5 text-left">
      <span
        aria-label={ACTIVITY_LABEL[entry.status]}
        title={ACTIVITY_LABEL[entry.status]}
        className={`size-2 shrink-0 rounded-full ${ACTIVITY_DOT[entry.status]}`}
      />
      <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-fg">{entry.command}</span>
      <span className="shrink-0 font-mono text-[9px] text-faint">
        {formatTime(entry.startedAtMs)} · {activityDuration(entry, now)}
      </span>
    </div>
  )
}

function ActivitySummary({
  entries,
  now,
  error,
  onViewAll,
}: {
  entries: McpActivityEntry[]
  now: number
  error: string | null
  onViewAll: () => void
}) {
  const recent = entries
    .filter((entry) => now - entry.startedAtMs <= RECENT_ACTIVITY_MS)
    .slice(0, 3)

  return (
    <div className="mb-3 overflow-hidden rounded border border-edge bg-panel">
      <button
        type="button"
        onClick={onViewAll}
        className="flex w-full items-center justify-between px-2 py-1.5 text-left hover:bg-elevated/70 focus-visible:outline focus-visible:outline-live"
      >
        <span className="text-[10px] font-medium uppercase tracking-wide text-faint">
          MCP activity
        </span>
        <span className="inline-flex items-center gap-1 text-[9px] text-muted">
          {entries.length > 0 ? `${entries.length} stored` : 'View history'}
          <ChevronIcon />
        </span>
      </button>

      {error ? (
        <div className="border-t border-edge px-2 py-2 text-[10px] text-error">
          Could not load MCP activity
        </div>
      ) : recent.length > 0 ? (
        <div className="border-t border-edge p-0.5">
          {recent.map((entry) => (
            <ActivityPreviewRow key={entry.id} entry={entry} now={now} />
          ))}
        </div>
      ) : (
        <div className="border-t border-edge px-2 py-2 text-left">
          <span className="block text-[10px] text-muted">No commands in the last 2 minutes</span>
          {entries[0] && (
            <span className="mt-0.5 block text-[9px] text-faint">
              Last activity at {formatTime(entries[0].startedAtMs)}
            </span>
          )}
        </div>
      )}
    </div>
  )
}

function ActivityHistory({
  entries,
  now,
  error,
  onBack,
  onSelect,
}: {
  entries: McpActivityEntry[]
  now: number
  error: string | null
  onBack: () => void
  onSelect: (id: number) => void
}) {
  return (
    <div className="flex h-[min(30rem,calc(100vh-3rem))] flex-col">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-edge px-3">
        <button
          type="button"
          title="Back to MCP settings"
          aria-label="Back to MCP settings"
          onClick={onBack}
          className="flex size-6 items-center justify-center rounded text-muted hover:bg-panel hover:text-fg focus-visible:outline focus-visible:outline-live"
        >
          <BackIcon />
        </button>
        <span className="text-[12px] font-medium text-fg">MCP activity</span>
        <span className="ml-auto text-[10px] text-faint">Current app run</span>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {error ? (
          <p role="alert" className="select-text p-4 text-[10px] text-error">
            {error}
          </p>
        ) : entries.length > 0 ? (
          entries.map((entry) => (
            <ActivityRow
              key={entry.id}
              entry={entry}
              now={now}
              onClick={() => onSelect(entry.id)}
            />
          ))
        ) : (
          <div className="flex h-full flex-col items-center justify-center px-6 text-center">
            <span className="mb-2 size-2 rounded-full bg-faint" />
            <p className="text-[11px] text-muted">No MCP commands yet</p>
            <p className="mt-1 max-w-64 text-[9px] leading-4 text-faint">
              Tool calls will appear here as soon as an MCP client sends them.
            </p>
          </div>
        )}
      </div>

      <div className="shrink-0 border-t border-edge px-3 py-2 text-[9px] text-faint">
        Keeping {entries.length} of {ACTIVITY_LIMIT} commands in memory
      </div>
    </div>
  )
}

function PayloadBlock({ title, value }: { title: string; value: unknown | null }) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<number | null>(null)
  const text = value === null ? '' : payloadText(value)

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current)
    },
    [],
  )

  const copy = async () => {
    if (value === null) return
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      if (timer.current !== null) window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => {
        setCopied(false)
        timer.current = null
      }, 2000)
    } catch {
      setCopied(false)
    }
  }

  return (
    <div>
      <div className="mb-1 flex h-6 items-center justify-between">
        <span className="text-[9px] font-medium uppercase tracking-wide text-faint">{title}</span>
        <CopyButton
          label={title}
          disabled={value === null}
          copied={copied}
          placement="inline"
          onClick={() => void copy()}
        />
      </div>
      <div className="relative rounded border border-edge bg-panel">
        {value === null ? (
          <div className="flex items-center gap-2 px-3 py-3 text-[10px] text-muted">
            <span className="size-1.5 animate-pulse rounded-full bg-warn" />
            Waiting for response…
          </div>
        ) : (
          <pre className="max-h-56 select-text overflow-auto whitespace-pre-wrap break-words p-3 font-mono text-[9px] leading-4 text-fg">
            {text}
          </pre>
        )}
      </div>
    </div>
  )
}

function ActivityDetail({
  entry,
  now,
  onBack,
}: {
  entry: McpActivityEntry | null
  now: number
  onBack: () => void
}) {
  const [view, setView] = useState<'pretty' | 'raw'>('pretty')
  const hasPretty = entry !== null && hasPrettyActivity(entry.command)

  useEffect(() => {
    setView(entry && hasPrettyActivity(entry.command) ? 'pretty' : 'raw')
  }, [entry?.id])

  return (
    <div className="flex h-[min(30rem,calc(100vh-3rem))] flex-col">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-edge px-3">
        <button
          type="button"
          title="Back to MCP activity"
          aria-label="Back to MCP activity"
          onClick={onBack}
          className="flex size-6 items-center justify-center rounded text-muted hover:bg-panel hover:text-fg focus-visible:outline focus-visible:outline-live"
        >
          <BackIcon />
        </button>
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-fg">
          {entry?.command ?? 'Command details'}
        </span>
        {entry && (
          <span className="inline-flex shrink-0 items-center gap-1.5 text-[9px] text-muted">
            <span className={`size-1.5 rounded-full ${ACTIVITY_DOT[entry.status]}`} />
            {ACTIVITY_LABEL[entry.status]}
          </span>
        )}
      </div>

      {entry ? (
        <div className="min-h-0 flex-1 select-none overflow-y-auto p-3">
          <div className="mb-3 flex flex-wrap items-center gap-x-4 gap-y-1 rounded border border-edge bg-panel px-3 py-2 text-[9px] text-muted">
            <span>
              Started <span className="font-mono text-fg">{formatTime(entry.startedAtMs)}</span>
            </span>
            <span>
              Duration <span className="font-mono text-fg">{activityDuration(entry, now)}</span>
            </span>
            {hasPretty && (
              <div className="ml-auto flex rounded bg-elevated p-0.5" aria-label="Command view">
                <button
                  type="button"
                  aria-pressed={view === 'pretty'}
                  onClick={() => setView('pretty')}
                  className={`rounded px-2 py-1 text-[8px] font-medium transition-colors ${view === 'pretty' ? 'bg-panel text-fg shadow-sm' : 'text-faint hover:text-muted'}`}
                >
                  Overview
                </button>
                <button
                  type="button"
                  aria-pressed={view === 'raw'}
                  onClick={() => setView('raw')}
                  className={`rounded px-2 py-1 text-[8px] font-medium transition-colors ${view === 'raw' ? 'bg-panel text-fg shadow-sm' : 'text-faint hover:text-muted'}`}
                >
                  RAW
                </button>
              </div>
            )}
          </div>
          {hasPretty && view === 'pretty' ? (
            <McpActivityPresentation entry={entry} />
          ) : (
            <div className="space-y-3">
              <PayloadBlock title="Request" value={entry.request} />
              <PayloadBlock title="Response" value={entry.response} />
            </div>
          )}
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center text-[10px] text-muted">
          This command is no longer in the activity buffer.
        </div>
      )}
    </div>
  )
}

export function McpControl() {
  const root = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const openRef = useRef(false)
  const [page, setPage] = useState<'main' | 'history' | 'detail'>('main')
  const [selectedActivityId, setSelectedActivityId] = useState<number | null>(null)
  const [activity, setActivity] = useState<McpActivityEntry[]>([])
  const [activityError, setActivityError] = useState<string | null>(null)
  const [now, setNow] = useState(Date.now())
  const activityRevision = useRef<number | undefined>(undefined)
  const activityRefreshing = useRef(false)
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

  const refreshActivity = async () => {
    if (activityRefreshing.current) return
    activityRefreshing.current = true
    try {
      const snapshot = await api.mcpActivity(activityRevision.current)
      activityRevision.current = snapshot.revision
      if (snapshot.entries !== null) setActivity(snapshot.entries)
      setActivityError(null)
    } catch (error) {
      setActivityError(`Could not load MCP activity: ${errorMessage(error)}`)
    } finally {
      activityRefreshing.current = false
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
    if (!open) return
    setNow(Date.now())
    void refreshActivity()
    const interval = window.setInterval(() => {
      setNow(Date.now())
      void refreshActivity()
    }, 750)
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
    setPage('main')
    setSelectedActivityId(null)
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
  const selectedActivity =
    selectedActivityId === null
      ? null
      : (activity.find((entry) => entry.id === selectedActivityId) ?? null)

  const openActivity = (id: number) => {
    setSelectedActivityId(id)
    setPage('detail')
  }

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
          aria-label={page === 'main' ? 'MCP server settings' : 'MCP command activity'}
          className={`absolute bottom-7 right-0 z-40 select-none overflow-hidden rounded border border-edge bg-elevated text-left shadow-2xl ${page === 'main' ? 'w-96 p-3' : 'w-[min(34rem,calc(100vw-1.5rem))]'}`}
        >
          {page === 'main' ? (
            <>
              <div className="mb-3 flex items-center justify-between">
                <span className="text-[12px] font-medium text-fg">
                  {info?.mode === 'remoteRepl' ? 'MCP · Remote REPL' : 'MCP server'}
                </span>
                <span className="inline-flex items-center gap-1.5 text-[11px] text-muted">
                  <span aria-hidden="true" className={`size-1.5 rounded-full ${DOT[status]}`} />
                  {LABEL[status]}
                </span>
              </div>

              <ActivitySummary
                entries={activity}
                now={now}
                error={activityError}
                onViewAll={() => setPage('history')}
              />

              {info?.mode === 'remoteRepl' && (
                <div className="mb-3 rounded border border-warn/40 bg-warn/10 p-2">
                  <p className="text-[11px] font-medium text-warn">Remote REPL only</p>
                  <p className="mt-1 select-text text-[10px] leading-4 text-muted">
                    This MCP exposes only <code className="text-fg">wot_exec</code> for Python 2.7
                    in the connected remote game. Client discovery and process control, logs,
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
                        integrationBusy !== null ||
                        integrationStatus?.chatgptCodex.available !== true
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
            </>
          ) : page === 'history' ? (
            <ActivityHistory
              entries={activity}
              now={now}
              error={activityError}
              onBack={() => setPage('main')}
              onSelect={openActivity}
            />
          ) : (
            <ActivityDetail entry={selectedActivity} now={now} onBack={() => setPage('history')} />
          )}
        </section>
      )}
    </div>
  )
}
