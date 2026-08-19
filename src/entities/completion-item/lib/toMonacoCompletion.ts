import type * as monaco from 'monaco-editor'
import type { Candidate } from '@/shared/api'

// monaco.languages.CompletionItemKind numeric literals (avoid a monaco runtime
// import in this entity): Function = 1, Class = 5, Property = 9.
const KIND_FUNCTION = 1 as monaco.languages.CompletionItemKind
const KIND_CLASS = 5 as monaco.languages.CompletionItemKind
const KIND_PROPERTY = 9 as monaco.languages.CompletionItemKind

function kindOf(c: Candidate): monaco.languages.CompletionItemKind {
  if (c.kind === 'class') return KIND_CLASS
  if (c.signature || c.kind === 'function') return KIND_FUNCTION
  return KIND_PROPERTY
}

// Order public names first, single-underscore "semi-private" next, and truly
// private members last: Python name-mangles a class's `__field` to
// `_ClassName__field`, and `__dunder__` are also noise for everyday use.
function privacyRank(name: string): number {
  if (!name.startsWith('_')) return 0
  if (/^_[A-Za-z]\w*__\w/.test(name) || name.startsWith('__')) return 2
  return 1
}

export function toMonacoCompletion(
  c: Candidate,
  range: monaco.IRange,
): monaco.languages.CompletionItem {
  // Show the typed signature inline next to the name (e.g.
  // "spaceLoadStatus(distance: float = -1.0) -> float").
  return {
    label: {
      label: c.name,
      // inline signature next to the name when known (e.g. "(x: int) -> bool")
      detail: c.signature ? ` ${c.signature}` : '',
      // right-aligned: the actual TYPE (function/class/int/Vector3/...), not 'live'
      description: c.kind ?? '',
    },
    kind: kindOf(c),
    insertText: c.name,
    detail: c.signature ?? c.kind ?? 'live',
    documentation: c.doc ?? undefined,
    range,
    sortText: `${privacyRank(c.name)}_${c.name.toLowerCase()}`,
  }
}
