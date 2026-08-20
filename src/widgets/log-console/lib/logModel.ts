import type * as monaco from 'monaco-editor'
import type { LogDecorationSpan, LogDocument } from './logDocument'

const NEVER_GROWS_AT_EDGES = 1 as monaco.editor.TrackedRangeStickiness
const LF = 0 as monaco.editor.EndOfLineSequence

export function toModelDecorations(
  model: Pick<monaco.editor.ITextModel, 'getPositionAt'>,
  spans: readonly LogDecorationSpan[],
  offset = 0,
): monaco.editor.IModelDeltaDecoration[] {
  return spans.map((span) => {
    const start = model.getPositionAt(offset + span.start)
    const end = model.getPositionAt(offset + span.end)
    return {
      range: {
        startLineNumber: start.lineNumber,
        startColumn: start.column,
        endLineNumber: end.lineNumber,
        endColumn: end.column,
      },
      options: {
        inlineClassName: span.className,
        stickiness: NEVER_GROWS_AT_EDGES,
      },
    }
  })
}

export function replaceLogDocument(
  model: Pick<monaco.editor.ITextModel, 'setEOL' | 'setValue' | 'getPositionAt'>,
  decorations: Pick<monaco.editor.IEditorDecorationsCollection, 'set'>,
  document: LogDocument,
): void {
  model.setValue(document.text)
  // An empty Monaco model uses the platform EOL (CRLF on Windows). Log
  // decoration offsets are calculated from an LF-normalized document, so fix
  // the model contract before the first live batch is appended.
  model.setEOL(LF)
  decorations.set(toModelDecorations(model, document.decorations))
}

export function appendLogDocument(
  model: Pick<monaco.editor.ITextModel, 'applyEdits' | 'getPositionAt' | 'getValueLength'>,
  decorations: Pick<monaco.editor.IEditorDecorationsCollection, 'append'>,
  document: LogDocument,
): void {
  if (!document.text) return

  const offset = model.getValueLength()
  const end = model.getPositionAt(offset)
  model.applyEdits([{
    range: {
      startLineNumber: end.lineNumber,
      startColumn: end.column,
      endLineNumber: end.lineNumber,
      endColumn: end.column,
    },
    text: document.text,
  }])
  decorations.append(toModelDecorations(model, document.decorations, offset))
}
