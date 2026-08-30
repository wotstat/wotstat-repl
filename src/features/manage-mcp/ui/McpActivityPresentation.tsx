import type { ReactNode } from 'react'
import type { McpActivityEntry } from '@/shared/api'
import {
  activityArguments,
  activityResponseError,
  activityResponseImage,
  activityStructuredContent,
  asArray,
  asBoolean,
  asNumber,
  asObject,
  asString,
} from '../lib/activityPresentation'

function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <span className="mb-1 block text-[9px] font-medium uppercase tracking-wide text-faint">
      {children}
    </span>
  )
}

function CodeBlock({
  label,
  text,
  tone = 'default',
}: {
  label: string
  text: string
  tone?: 'default' | 'error'
}) {
  return (
    <div>
      <SectionLabel>{label}</SectionLabel>
      <pre
        className={`max-h-56 select-text overflow-auto whitespace-pre-wrap break-words rounded border border-edge bg-panel p-3 font-mono text-[10px] leading-4 ${tone === 'error' ? 'text-error' : 'text-fg'}`}
      >
        {text}
      </pre>
    </div>
  )
}

function Notice({
  tone,
  children,
}: {
  tone: 'pending' | 'error' | 'success'
  children: ReactNode
}) {
  const colors = {
    pending: 'border-warn/30 bg-warn/5 text-warn',
    error: 'border-error/30 bg-error/5 text-error',
    success: 'border-ok/30 bg-ok/5 text-ok',
  }
  const dot = { pending: 'bg-warn animate-pulse', error: 'bg-error', success: 'bg-ok' }
  return (
    <div className={`flex items-start gap-2 rounded border px-3 py-2 text-[10px] ${colors[tone]}`}>
      <span className={`mt-1 size-1.5 shrink-0 rounded-full ${dot[tone]}`} />
      <span className="select-text leading-4">{children}</span>
    </div>
  )
}

function ResponseState({ entry }: { entry: McpActivityEntry }) {
  if (entry.response === null) return <Notice tone="pending">Waiting for the MCP response…</Notice>
  const error = activityResponseError(entry.response)
  if (error) return <Notice tone="error">{error}</Notice>
  return null
}

function SummaryCard({
  icon,
  title,
  detail,
  children,
}: {
  icon: ReactNode
  title: string
  detail?: string | null
  children?: ReactNode
}) {
  return (
    <div className="rounded border border-edge bg-panel p-3">
      <div className="flex items-start gap-3">
        <div className="flex size-8 shrink-0 items-center justify-center rounded bg-elevated text-muted">
          {icon}
        </div>
        <div className="min-w-0 flex-1">
          <p className="select-text text-[11px] font-medium text-fg">{title}</p>
          {detail && (
            <p className="mt-0.5 select-text break-all text-[9px] leading-4 text-muted">{detail}</p>
          )}
          {children}
        </div>
      </div>
    </div>
  )
}

function TerminalIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="none" aria-hidden="true" className="size-4 stroke-current">
      <path
        d="m4 6 3 3-3 3M9 13h6"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <rect x="2.5" y="3.5" width="15" height="13" rx="2" strokeWidth="1.2" />
    </svg>
  )
}

function ScreenshotIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="none" aria-hidden="true" className="size-5 stroke-current">
      <rect x="2.5" y="4.5" width="15" height="11" rx="2" strokeWidth="1.2" />
      <circle cx="10" cy="10" r="3" strokeWidth="1.2" />
      <path d="M6 4.5 7 3h6l1 1.5" strokeWidth="1.2" strokeLinecap="round" />
    </svg>
  )
}

function GameIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="none" aria-hidden="true" className="size-4 stroke-current">
      <path
        d="M6 7h8a3 3 0 0 1 2.8 2l1.1 3.2a2 2 0 0 1-3.2 2.2L13 13H7l-1.7 1.4a2 2 0 0 1-3.2-2.2L3.2 9A3 3 0 0 1 6 7Z"
        strokeWidth="1.2"
      />
      <path d="M6 9v3M4.5 10.5h3M13.5 10h.01M15.5 12h.01" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  )
}

function LogIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="none" aria-hidden="true" className="size-4 stroke-current">
      <path d="M5 5h10M5 9h10M5 13h7" strokeWidth="1.3" strokeLinecap="round" />
      <rect x="2.5" y="2.5" width="15" height="15" rx="2" strokeWidth="1.2" />
    </svg>
  )
}

function MouseIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="none" aria-hidden="true" className="size-4 stroke-current">
      <rect x="5" y="2.5" width="10" height="15" rx="5" strokeWidth="1.2" />
      <path d="M10 2.5V8M5 8h10" strokeWidth="1.2" />
    </svg>
  )
}

function MouseViewportMap({
  x,
  y,
  width,
  height,
}: {
  x: number
  y: number
  width: number
  height: number
}) {
  const left = Math.min(100, Math.max(0, (x / width) * 100))
  const top = Math.min(100, Math.max(0, (y / height) * 100))
  const roundedWidth = Math.round(width)
  const roundedHeight = Math.round(height)
  const roundedX = Math.round(x)
  const roundedY = Math.round(y)

  return (
    <div>
      <SectionLabel>Game viewport</SectionLabel>
      <div
        className="relative mx-auto w-full overflow-hidden rounded border border-edge bg-canvas"
        style={{ aspectRatio: `${width} / ${height}` }}
        aria-label={`Mouse position ${roundedX}, ${roundedY} in a ${roundedWidth} by ${roundedHeight} game viewport`}
      >
        <span className="absolute inset-y-0 left-1/3 border-l border-edge/70" />
        <span className="absolute inset-y-0 left-2/3 border-l border-edge/70" />
        <span className="absolute inset-x-0 top-1/3 border-t border-edge/70" />
        <span className="absolute inset-x-0 top-2/3 border-t border-edge/70" />
        <span
          className="absolute size-2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-live shadow-[0_0_0_2px_var(--color-canvas)]"
          style={{ left: `${left}%`, top: `${top}%` }}
        >
          <span className="absolute -inset-2 animate-ping rounded-full bg-live/45" />
        </span>
      </div>
      <div className="mt-1.5 text-right text-[9px]">
        <span className="font-mono text-faint">
          {roundedWidth} × {roundedHeight} · {roundedX}, {roundedY}
        </span>
      </div>
    </div>
  )
}

function KeyboardIcon() {
  return (
    <svg viewBox="0 0 20 20" fill="none" aria-hidden="true" className="size-4 stroke-current">
      <rect x="2" y="4.5" width="16" height="11" rx="2" strokeWidth="1.2" />
      <path
        d="M5 8h.01M8 8h.01M11 8h.01M14 8h.01M5 11h.01M8 11h.01M11 11h.01M14 11h.01M6 13.5h8"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  )
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function formatTimeout(value: unknown, fallback: string): string {
  const timeout = asNumber(value)
  return timeout === null ? fallback : `${timeout.toLocaleString()} ms`
}

function humanStatus(value: unknown): string {
  const status = asString(value)
  if (!status) return 'Unknown'
  return status.replaceAll('_', ' ').replace(/^./, (character) => character.toUpperCase())
}

function CapabilityBadges({ value }: { value: unknown }) {
  const capabilities = asObject(value)
  if (!capabilities) return null
  const enabled = Object.entries(capabilities)
    .filter(([, available]) => available === true)
    .map(([name]) => name)
  if (enabled.length === 0) return null
  return (
    <div className="mt-2 flex flex-wrap gap-1">
      {enabled.map((name) => (
        <span
          key={name}
          className="rounded bg-elevated px-1.5 py-0.5 text-[8px] uppercase text-muted"
        >
          {name}
        </span>
      ))}
    </div>
  )
}

function ClientCard({ value }: { value: unknown }) {
  const client = asObject(value)
  if (!client) return null
  const path = asString(client.path)
  const executable = asString(client.exe)
  const version = asString(client.version)
  const pid = asNumber(client.pid)
  const agentVersion = asString(client.agentVersion)
  return (
    <SummaryCard icon={<GameIcon />} title={executable ?? 'World of Tanks client'} detail={path}>
      <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[9px] text-muted">
        {version && (
          <span>
            Version <span className="font-mono text-fg">{version}</span>
          </span>
        )}
        <span>
          Process <span className="text-fg">{humanStatus(client.processStatus)}</span>
        </span>
        {client.agentStatus != null && (
          <span>
            Agent <span className="text-fg">{humanStatus(client.agentStatus)}</span>
          </span>
        )}
        {pid !== null && (
          <span>
            PID <span className="font-mono text-fg">{pid}</span>
          </span>
        )}
        {agentVersion && (
          <span>
            Agent <span className="font-mono text-fg">v{agentVersion}</span>
          </span>
        )}
      </div>
      <CapabilityBadges value={client.capabilities} />
    </SummaryCard>
  )
}

function ExecPresentation({ entry }: { entry: McpActivityEntry }) {
  const args = activityArguments(entry.request)
  const result = activityStructuredContent(entry.response)
  const outputs = [
    { label: 'Result', value: asString(result?.repr), tone: 'default' as const },
    { label: 'Stdout', value: asString(result?.stdout), tone: 'default' as const },
    { label: 'Stderr', value: asString(result?.stderr), tone: 'error' as const },
    { label: 'Exception', value: asString(result?.exception), tone: 'error' as const },
  ].filter((output) => output.value != null && output.value.length > 0)

  return (
    <div className="space-y-3">
      <CodeBlock label="Python" text={asString(args.code) ?? ''} />
      <ResponseState entry={entry} />
      {outputs.map((output) => (
        <CodeBlock
          key={output.label}
          label={output.label}
          text={output.value!}
          tone={output.tone}
        />
      ))}
    </div>
  )
}

function ScreenshotPresentation({ entry }: { entry: McpActivityEntry }) {
  const result = activityStructuredContent(entry.response)
  const error = activityResponseError(entry.response)
  const image = activityResponseImage(entry.response)
  const dimensions =
    asNumber(result?.width) !== null && asNumber(result?.height) !== null
      ? `${asNumber(result?.width)} × ${asNumber(result?.height)}`
      : null
  const size = asNumber(result?.size)
  const detail = [dimensions, size === null ? null : formatBytes(size)].filter(Boolean).join(' · ')
  return (
    <div className="space-y-3">
      {image ? (
        <figure className="overflow-hidden rounded border border-edge bg-canvas">
          <img
            src={`data:${image.mimeType};base64,${image.data}`}
            alt="World of Tanks screenshot preview"
            loading="lazy"
            decoding="async"
            draggable={false}
            className="max-h-56 w-full object-contain"
          />
          {detail && (
            <figcaption className="border-t border-edge bg-panel px-3 py-1.5 text-center font-mono text-[8px] text-faint">
              {detail}
            </figcaption>
          )}
        </figure>
      ) : (
        <div className="flex min-h-32 flex-col items-center justify-center rounded border border-edge bg-panel text-muted">
          <div
            className={
              error
                ? 'text-error'
                : entry.response === null
                  ? 'animate-pulse text-warn'
                  : 'text-muted'
            }
          >
            <ScreenshotIcon />
          </div>
          <span className="mt-2 text-[10px] text-fg">
            {error
              ? 'Screenshot failed'
              : entry.response === null
                ? 'Capturing screenshot…'
                : 'Screenshot captured'}
          </span>
          {!error && detail && (
            <span className="mt-1 font-mono text-[8px] text-faint">{detail}</span>
          )}
        </div>
      )}
      {error && <Notice tone="error">{error}</Notice>}
    </div>
  )
}

function ListClientsPresentation({ entry }: { entry: McpActivityEntry }) {
  const result = activityStructuredContent(entry.response)
  const clients = asArray(result?.clients)
  return (
    <div className="space-y-2">
      <ResponseState entry={entry} />
      {entry.response !== null &&
        !activityResponseError(entry.response) &&
        clients.length === 0 && (
          <SummaryCard icon={<GameIcon />} title="No World of Tanks clients found" />
        )}
      {clients.map((client, index) => (
        <ClientCard key={asString(asObject(client)?.path) ?? index} value={client} />
      ))}
    </div>
  )
}

function ReadLogPresentation({ entry }: { entry: McpActivityEntry }) {
  const args = activityArguments(entry.request)
  const result = activityStructuredContent(entry.response)
  const logs = asArray(result?.entries)
  const cursor = asNumber(args.cursor)
  const limit = asNumber(args.limit) ?? 200
  const wait = asNumber(args.wait_ms) ?? 0
  return (
    <div className="space-y-3">
      <SummaryCard
        icon={<LogIcon />}
        title={cursor === null ? 'Read latest game logs' : `Read logs after #${cursor}`}
        detail={`Up to ${limit} entries${wait > 0 ? ` · wait ${wait} ms` : ''}`}
      />
      <ResponseState entry={entry} />
      {entry.response !== null && !activityResponseError(entry.response) && logs.length === 0 && (
        <p className="py-4 text-center text-[10px] text-muted">No new log entries</p>
      )}
      {logs.length > 0 && (
        <div>
          <div className="mb-1 flex items-center justify-between">
            <SectionLabel>Log output</SectionLabel>
            {asBoolean(result?.truncated) && (
              <span className="text-[8px] text-warn">Truncated</span>
            )}
          </div>
          <div className="max-h-64 select-text overflow-auto rounded border border-edge bg-panel font-mono text-[9px] leading-4">
            {logs.map((item, index) => {
              const log = asObject(item)
              const level = asString(log?.level)
              const stream = asString(log?.stream) ?? 'log'
              const color = level === 'error' || stream === 'stderr' ? 'text-error' : 'text-fg'
              return (
                <div
                  key={asNumber(log?.sequence) ?? index}
                  className="flex gap-2 border-b border-edge/60 px-2 py-1 last:border-0"
                >
                  <span className="w-12 shrink-0 text-faint">{level ?? stream}</span>
                  <span className={`whitespace-pre-wrap break-words ${color}`}>
                    {asString(log?.text) ?? ''}
                  </span>
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}

function StartClientPresentation({ entry }: { entry: McpActivityEntry }) {
  const args = activityArguments(entry.request)
  const waitUntilReady = asBoolean(args.wait_until_ready) ?? true
  return (
    <div className="space-y-3">
      <SummaryCard
        icon={<GameIcon />}
        title={waitUntilReady ? 'Start client and wait until ready' : 'Start client'}
        detail={asString(args.game_dir)}
      >
        <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[9px] text-muted">
          {asString(args.replay_path) && (
            <span className="break-all">
              Replay <span className="text-fg">{asString(args.replay_path)}</span>
            </span>
          )}
          <span>
            Ready timeout{' '}
            <span className="font-mono text-fg">
              {formatTimeout(args.ready_timeout_ms, 'unlimited')}
            </span>
          </span>
        </div>
      </SummaryCard>
      <ResponseState entry={entry} />
      {!activityResponseError(entry.response) && (
        <ClientCard value={activityStructuredContent(entry.response)} />
      )}
    </div>
  )
}

function CloseClientPresentation({ entry }: { entry: McpActivityEntry }) {
  const args = activityArguments(entry.request)
  const result = activityStructuredContent(entry.response)
  const stillRunning = asBoolean(result?.stillRunning)
  return (
    <div className="space-y-3">
      <SummaryCard
        icon={<GameIcon />}
        title="Gracefully close the active client"
        detail={`Wait up to ${formatTimeout(args.timeout_ms, '10,000 ms')}`}
      />
      <ResponseState entry={entry} />
      {!activityResponseError(entry.response) && stillRunning !== null && (
        <Notice tone={stillRunning ? 'pending' : 'success'}>
          {stillRunning
            ? 'Close requested, but the client is still running.'
            : 'Client closed successfully.'}
        </Notice>
      )}
      <ClientCard value={result?.client} />
    </div>
  )
}

function KillClientPresentation({ entry }: { entry: McpActivityEntry }) {
  return (
    <div className="space-y-3">
      <SummaryCard
        icon={<GameIcon />}
        title="Force-stop the active client"
        detail="The saved process identity was verified before termination."
      />
      <ResponseState entry={entry} />
      {!activityResponseError(entry.response) && (
        <ClientCard value={activityStructuredContent(entry.response)} />
      )}
    </div>
  )
}

function MousePresentation({ entry }: { entry: McpActivityEntry }) {
  const args = activityArguments(entry.request)
  const result = activityStructuredContent(entry.response)
  const action = humanStatus(args.action)
  const button = asString(args.button)
  const x = asNumber(result?.x) ?? asNumber(args.x)
  const y = asNumber(result?.y) ?? asNumber(args.y)
  const width = asNumber(result?.width)
  const height = asNumber(result?.height)
  const wheel = asNumber(args.wheel_delta)
  const modifiers = asArray(args.modifiers).filter(
    (value): value is string => typeof value === 'string',
  )
  const detail = [
    button,
    wheel === null ? null : `delta ${wheel}`,
    modifiers.length > 0 ? modifiers.join(' + ') : null,
  ]
    .filter(Boolean)
    .join(' · ')
  return (
    <div className="space-y-3">
      <SummaryCard icon={<MouseIcon />} title={`${action} mouse input`} detail={detail} />
      <ResponseState entry={entry} />
      {!activityResponseError(entry.response) &&
        x !== null &&
        y !== null &&
        width !== null &&
        height !== null &&
        width > 0 &&
        height > 0 && <MouseViewportMap x={x} y={y} width={width} height={height} />}
      {!activityResponseError(entry.response) && asBoolean(result?.delivered) === true && (
        <Notice tone="success">Virtual mouse event delivered to the game.</Notice>
      )}
    </div>
  )
}

function KeyboardPresentation({ entry }: { entry: McpActivityEntry }) {
  const args = activityArguments(entry.request)
  const result = activityStructuredContent(entry.response)
  const key = asString(args.key) ?? 'Unknown key'
  const action = humanStatus(args.action)
  const character = asString(args.character)
  const modifiers = asArray(args.modifiers).filter(
    (value): value is string => typeof value === 'string',
  )
  const combination = [...modifiers, key].join(' + ')
  return (
    <div className="space-y-3">
      <SummaryCard
        icon={<KeyboardIcon />}
        title={`${action} keyboard input`}
        detail={character ? `Character “${character}”` : null}
      >
        <span className="mt-2 inline-block rounded border border-edge bg-elevated px-2 py-1 font-mono text-[10px] text-fg shadow-sm">
          {combination}
        </span>
      </SummaryCard>
      <ResponseState entry={entry} />
      {!activityResponseError(entry.response) && asBoolean(result?.delivered) === true && (
        <Notice tone="success">Virtual keyboard event delivered to the game.</Notice>
      )}
    </div>
  )
}

export function McpActivityPresentation({ entry }: { entry: McpActivityEntry }) {
  switch (entry.command) {
    case 'wot_exec':
      return <ExecPresentation entry={entry} />
    case 'wot_screenshot':
      return <ScreenshotPresentation entry={entry} />
    case 'wot_list_clients':
      return <ListClientsPresentation entry={entry} />
    case 'wot_read_log':
      return <ReadLogPresentation entry={entry} />
    case 'wot_start_client':
      return <StartClientPresentation entry={entry} />
    case 'wot_close_client':
      return <CloseClientPresentation entry={entry} />
    case 'wot_kill_client':
      return <KillClientPresentation entry={entry} />
    case 'wot_mouse':
      return <MousePresentation entry={entry} />
    case 'wot_keyboard':
      return <KeyboardPresentation entry={entry} />
    default:
      return (
        <SummaryCard
          icon={<TerminalIcon />}
          title={entry.command}
          detail="No readable presentation is available for this command."
        />
      )
  }
}
