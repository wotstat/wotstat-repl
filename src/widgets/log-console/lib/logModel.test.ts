import { describe, expect, test } from 'bun:test'
import type * as monaco from 'monaco-editor'
import { projectLogLines } from './logDocument'
import { appendLogDocument, replaceLogDocument } from './logModel'

class WindowsModel {
  private value = ''
  private eol = '\r\n'

  setValue(value: string): void {
    this.eol = value.includes('\n') ? '\n' : '\r\n'
    this.value = value.replace(/\r\n?|\n/g, this.eol)
  }

  setEOL(eol: monaco.editor.EndOfLineSequence): void {
    const nextEol = eol === 0 ? '\n' : '\r\n'
    this.value = this.value.replace(/\r\n?|\n/g, nextEol)
    this.eol = nextEol
  }

  getValueLength(): number {
    return this.value.length
  }

  applyEdits(edits: readonly monaco.editor.IIdentifiedSingleEditOperation[]): void {
    for (const edit of edits) {
      this.value += (edit.text ?? '').replace(/\r\n?|\n/g, this.eol)
    }
  }

  getPositionAt(rawOffset: number): monaco.Position {
    const offset = Math.max(0, Math.min(rawOffset, this.value.length))
    const lines = this.value.slice(0, offset).split(this.eol)
    return { lineNumber: lines.length, column: lines.at(-1)!.length + 1 } as monaco.Position
  }
}

describe('log model updates', () => {
  test('keeps the first appended history aligned before and after a settings rebuild', () => {
    const history = [{
      stream: 'log',
      level: 'logInfo',
      text: '[startup] history\n',
    }]
    const error = {
      stream: 'log',
      level: 'logError',
      timestamp: '2026-08-20 19:03:41.690',
      source: 'Main',
      text: '[ERROR] request failed\n',
    }
    const document = projectLogLines(
      [...history, error],
      { showTimestamp: true, showLevel: true, showSource: true },
    )
    const model = new WindowsModel()
    let appended: readonly monaco.editor.IModelDeltaDecoration[] = []
    let rebuilt: readonly monaco.editor.IModelDeltaDecoration[] = []

    replaceLogDocument(
      model as never,
      { set: () => [] },
      { text: '', decorations: [] },
    )
    appendLogDocument(
      model as never,
      { append: (next) => { appended = next; return [] } },
      document,
    )

    const expectedRange = {
      startLineNumber: 2,
      startColumn: '2026-08-20 19:03:41.690: ERROR: Main: [ERROR] '.length + 1,
      endLineNumber: 2,
      endColumn: '2026-08-20 19:03:41.690: ERROR: Main: [ERROR] request failed'.length + 1,
    }
    expect(appended).toHaveLength(1)
    expect(appended[0]?.range).toEqual(expectedRange)

    replaceLogDocument(
      model as never,
      { set: (next) => { rebuilt = next; return [] } },
      document,
    )
    expect(rebuilt[0]?.range).toEqual(expectedRange)
  })
})
