import { describe, expect, test } from 'bun:test'
import { visibleLogLines } from './logDisplay'

describe('visibleLogLines', () => {
  test('always combines python.log fallback with lossless live output', () => {
    const replLog = { stream: 'log', level: 'logInfo', text: '[REPL] ready\n' }
    const file = {
      stream: 'python_log',
      level: 'INFO',
      timestamp: '2026-08-20 16:52:33.304',
      text: 'Renou_EU\n',
    }
    const stdout = {
      stream: 'stdout',
      level: 'INFO',
      timestamp: '2026-08-20 16:52:33.305',
      text: 'Renou_EU\n',
    }
    const stderr = {
      stream: 'stderr',
      level: 'ERROR',
      timestamp: '2026-08-20 16:52:34.001',
      text: 'repl error\n',
    }

    expect(visibleLogLines([replLog, file, stdout, stderr], new Set(), '', true)).toEqual([
      replLog,
      file,
      stdout,
      stderr,
    ])
  })

  test('can hide executed code without hiding its result or errors', () => {
    const lines = [
      { stream: 'input', text: '>>> 1 + 1\n' },
      { stream: 'result', text: '2\n' },
      { stream: 'system', text: 'agent online\n' },
      { stream: 'stderr', text: 'request failed\n' },
    ]

    expect(visibleLogLines(lines, new Set(), '', true)).toEqual(lines)
    expect(visibleLogLines(lines, new Set(), '', false)).toEqual(lines.slice(1))
  })
})
