import type { LogLine } from '@/shared/api'

export const SEVERITIES = ['INFO', 'DEBUG', 'NOTICE', 'WARNING', 'ERROR', 'CRITICAL'] as const
export type Severity = (typeof SEVERITIES)[number]

const ALIASES: Record<string, Severity> = {
  INFO: 'INFO',
  DEBUG: 'DEBUG',
  TRACE: 'DEBUG',
  NOTICE: 'NOTICE',
  HOOK: 'NOTICE',
  WARNING: 'WARNING',
  ERROR: 'ERROR',
  CRITICAL: 'CRITICAL',
  HACK: 'CRITICAL',
}

const INLINE = /^(?:\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+:?\s+)?(INFO|WARNING|ERROR|DEBUG|NOTICE|CRITICAL|TRACE|HACK|HOOK):/

// REPL I/O and plain stdout carry no severity; they are user-facing and must never be
// hidden by the level filter, so they resolve to null ("always shown").
const ALWAYS_SHOWN = new Set(['input', 'result', 'system'])

export function lineSeverity(line: LogLine): Severity | null {
  if (ALWAYS_SHOWN.has(line.stream)) return null

  if (line.level) {
    const token = line.level.replace(/^log/, '').toUpperCase()
    return ALIASES[token] ?? null
  }

  const m = INLINE.exec(line.text)
  if (m) return ALIASES[m[1]] ?? null

  if (line.stream === 'stderr') return 'ERROR'
  return null
}

export function matchesFilter(line: LogLine, hidden: ReadonlySet<Severity>): boolean {
  const sev = lineSeverity(line)
  if (sev === null) return true
  return !hidden.has(sev)
}

export function matchesSearch(line: LogLine, needle: string): boolean {
  if (!needle) return true
  const normalizedNeedle = needle.toLowerCase()
  return [line.timestamp, line.level, line.source, line.text]
    .some((value) => value?.toLowerCase().includes(normalizedNeedle))
}
