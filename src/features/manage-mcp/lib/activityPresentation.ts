export type JsonObject = Record<string, unknown>

export const PRETTY_ACTIVITY_COMMANDS = new Set([
  'wot_exec',
  'wot_screenshot',
  'wot_list_clients',
  'wot_read_log',
  'wot_start_client',
  'wot_close_client',
  'wot_kill_client',
  'wot_mouse',
  'wot_keyboard',
])

export interface ActivityNavigation {
  position: number
  total: number
  previousId: number | null
  nextId: number | null
}

export function getActivityNavigation(
  entries: readonly { id: number }[],
  selectedId: number | null,
): ActivityNavigation {
  const total = entries.length
  const selectedIndex =
    selectedId === null ? -1 : entries.findIndex((entry) => entry.id === selectedId)

  if (selectedIndex === -1) {
    return { position: 0, total, previousId: null, nextId: null }
  }

  return {
    position: total - selectedIndex,
    total,
    previousId: entries[selectedIndex + 1]?.id ?? null,
    nextId: entries[selectedIndex - 1]?.id ?? null,
  }
}

export function asObject(value: unknown): JsonObject | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as JsonObject)
    : null
}

export function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : []
}

export function asString(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

export function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

export function asBoolean(value: unknown): boolean | null {
  return typeof value === 'boolean' ? value : null
}

export function hasPrettyActivity(command: string): boolean {
  return PRETTY_ACTIVITY_COMMANDS.has(command)
}

export function activityArguments(request: unknown): JsonObject {
  const params = asObject(asObject(request)?.params)
  return asObject(params?.arguments) ?? {}
}

function activityResult(response: unknown): JsonObject | null {
  return asObject(asObject(response)?.result)
}

export function activityStructuredContent(response: unknown): JsonObject | null {
  return asObject(activityResult(response)?.structuredContent)
}

export function activityResponseText(response: unknown): string | null {
  const content = asArray(activityResult(response)?.content)
  for (const block of content) {
    const item = asObject(block)
    if (item?.type === 'text') {
      const text = asString(item.text)
      if (text) return text
    }
  }
  return null
}

export function activityResponseImage(
  response: unknown,
): { data: string; mimeType: string } | null {
  const content = asArray(activityResult(response)?.content)
  for (const block of content) {
    const item = asObject(block)
    if (item?.type !== 'image') continue
    const data = asString(item.data)
    const mimeType = asString(item.mimeType)
    if (data && mimeType?.startsWith('image/')) return { data, mimeType }
  }
  return null
}

export function activityResponseError(response: unknown): string | null {
  const envelope = asObject(response)
  const protocolError = asObject(envelope?.error)
  if (protocolError) return asString(protocolError.message) ?? 'MCP protocol error'

  const result = activityResult(response)
  if (result?.isError === true) return activityResponseText(response) ?? 'MCP command failed'
  return null
}
