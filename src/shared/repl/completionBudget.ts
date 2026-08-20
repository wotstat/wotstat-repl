import {
  COMPLETION_BUDGET_STORAGE_KEY,
  DEFAULT_COMPLETION_BUDGET,
  MAX_COMPLETION_BUDGET,
} from '@/shared/config'
import { loadState } from '@/shared/lib/storage'

export function completionBudget(): number {
  const value = loadState<unknown>(
    COMPLETION_BUDGET_STORAGE_KEY,
    DEFAULT_COMPLETION_BUDGET,
  )
  return typeof value === 'number' && Number.isInteger(value)
    ? Math.min(MAX_COMPLETION_BUDGET, Math.max(0, value))
    : DEFAULT_COMPLETION_BUDGET
}
