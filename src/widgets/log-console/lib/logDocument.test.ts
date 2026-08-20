import { describe, expect, test } from 'bun:test'
import { projectLogLines } from './logDocument'

describe('projectLogLines', () => {
  test('normalizes newlines and preserves chunk order', () => {
    const document = projectLogLines([
      { stream: 'stdout', text: 'one\r\ntwo\r' },
      { stream: 'stdout', text: 'three\n' },
    ])

    expect(document.text).toBe('one\ntwo\nthree\n')
    expect(document.decorations).toEqual([])
  })

  test('decorates REPL and system chunks by their stream', () => {
    const document = projectLogLines([
      { stream: 'input', text: '>>> value\n' },
      { stream: 'result', text: '42\n' },
      { stream: 'system', text: 'connected\n' },
    ])

    expect(document.decorations).toEqual([
      { start: 0, end: 10, className: 'console-log-input' },
      { start: 10, end: 13, className: 'console-log-result' },
      { start: 13, end: 23, className: 'console-log-system' },
    ])
  })

  test('tints only message text for structured warning frames', () => {
    const document = projectLogLines([
      { stream: 'log', level: 'logWarning', text: '[BigWorld] first\nsecond\n' },
    ])

    expect(document.decorations).toEqual([
      { start: 11, end: 16, className: 'console-log-warning' },
      { start: 17, end: 23, className: 'console-log-warning' },
    ])
  })

  test('treats plain stderr as an error but leaves info frames neutral', () => {
    const document = projectLogLines([
      { stream: 'stderr', text: 'failure\n' },
      { stream: 'log', level: 'logInfo', text: '[BigWorld] ready\n' },
    ])

    expect(document.decorations).toEqual([
      { start: 0, end: 8, className: 'console-log-error' },
    ])
  })

  test('projects optional timestamp and level metadata without changing payload text', () => {
    const document = projectLogLines(
      [{
        stream: 'log',
        level: 'logInfo',
        timestamp: '2026-08-19 21:48:32.033',
        text: '[web.cache.web_cache] WebDownloader destroyed\n',
      }],
      { showTimestamp: true, showLevel: true, showSource: false },
    )

    expect(document.text).toBe(
      '2026-08-19 21:48:32.033: INFO: [web.cache.web_cache] WebDownloader destroyed\n',
    )
  })

  test('renders print output with the same metadata shape as python.log', () => {
    const document = projectLogLines(
      [{
        stream: 'stdout',
        level: 'INFO',
        timestamp: '2026-08-20 05:17:07.011',
        source: 'Main',
        text: 'Renou_EU\n',
      }],
      { showTimestamp: true, showLevel: true, showSource: true },
    )

    expect(document.text).toBe('2026-08-20 05:17:07.011: INFO: Main: Renou_EU\n')
  })
})
