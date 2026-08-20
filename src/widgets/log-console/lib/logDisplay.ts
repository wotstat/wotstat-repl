import type { LogLine } from '@/shared/api'
import { matchesFilter, matchesSearch, type Severity } from './severity'

export function visibleLogLines(
  lines: readonly LogLine[],
  hidden: ReadonlySet<Severity>,
  filter: string,
  showInput: boolean,
): LogLine[] {
  return lines.filter((line) => (showInput || line.stream !== 'input')
    && matchesFilter(line, hidden)
    && matchesSearch(line, filter))
}
