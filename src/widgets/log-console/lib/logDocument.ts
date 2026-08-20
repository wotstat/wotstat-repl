import type { LogLine } from '@/shared/api'
import { lineSeverity } from './severity'

export interface LogDecorationSpan {
  start: number
  end: number
  className: string
}

export interface LogDocument {
  text: string
  decorations: LogDecorationSpan[]
}

export interface LogDisplayOptions {
  showTimestamp: boolean
  showLevel: boolean
  showSource: boolean
}

export const DEFAULT_LOG_DISPLAY_OPTIONS: LogDisplayOptions = {
  showTimestamp: false,
  showLevel: false,
  showSource: false,
}

const STRUCTURED_LINE = /^(?<ts>\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+:?\s+)?(?<lvl>(?:INFO|WARNING|ERROR|DEBUG|NOTICE|CRITICAL|TRACE|HACK|HOOK):?\s+)?(?<source>(?:[^\s:\[][^:\n]*):\s+)?(?<cat>(?:\[[^\]\n,]*\])+\s*)?(?<msg>.*)$/

function normalizeNewlines(text: string): string {
  return text.replace(/\r\n?|\n/g, '\n')
}

function levelLabel(level: string | null | undefined): string | null {
  if (!level) return null
  return level.replace(/^log/i, '').toUpperCase()
}

function displayText(line: LogLine, options: LogDisplayOptions): string {
  const text = normalizeNewlines(line.text)
  if (line.stream === 'input' || line.stream === 'result' || line.stream === 'system') {
    return text
  }

  const prefix: string[] = []
  if (options.showTimestamp && line.timestamp) prefix.push(`${line.timestamp}:`)
  const level = levelLabel(line.level)
  if (options.showLevel && level) prefix.push(`${level}:`)
  if (options.showSource && line.source) prefix.push(`${line.source}:`)
  return prefix.length > 0 ? `${prefix.join(' ')} ${text}` : text
}

function messageSpans(text: string, offset: number, className: string): LogDecorationSpan[] {
  const spans: LogDecorationSpan[] = []
  let lineStart = 0

  while (lineStart < text.length) {
    const nextNewline = text.indexOf('\n', lineStart)
    const lineEnd = nextNewline === -1 ? text.length : nextNewline
    const line = text.slice(lineStart, lineEnd)
    const match = STRUCTURED_LINE.exec(line)
    const message = match?.groups?.msg

    if (message) {
      const messageStart = lineEnd - message.length
      spans.push({
        start: offset + messageStart,
        end: offset + lineEnd,
        className,
      })
    }

    if (nextNewline === -1) break
    lineStart = nextNewline + 1
  }

  return spans
}

function decorationSpans(line: LogLine, text: string, offset: number): LogDecorationSpan[] {
  if (!text) return []

  const wholeChunkClass =
    line.stream === 'input'
      ? 'console-log-input'
      : line.stream === 'result'
        ? 'console-log-result'
        : line.stream === 'system'
          ? 'console-log-system'
          : line.stream === 'stderr' && !line.level
            ? 'console-log-error'
            : null

  if (wholeChunkClass) {
    return [{ start: offset, end: offset + text.length, className: wholeChunkClass }]
  }

  if (!line.level) return []
  const severity = lineSeverity(line)
  if (severity !== 'WARNING' && severity !== 'ERROR' && severity !== 'CRITICAL') return []

  return messageSpans(text, offset, `console-log-${severity.toLowerCase()}`)
}

export function projectLogLines(
  lines: readonly LogLine[],
  options: LogDisplayOptions = DEFAULT_LOG_DISPLAY_OPTIONS,
): LogDocument {
  const parts: string[] = []
  const decorations: LogDecorationSpan[] = []
  let offset = 0

  for (const line of lines) {
    const text = displayText(line, options)
    decorations.push(...decorationSpans(line, text, offset))
    parts.push(text)
    offset += text.length
  }

  return { text: parts.join(''), decorations }
}
