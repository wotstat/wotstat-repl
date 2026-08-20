import type * as monaco from 'monaco-editor'

export const LOG_LANGUAGE_ID = 'wms-log'

let registered = false

const TIMESTAMP = String.raw`\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+:?\s+`
const SOURCE = String.raw`[^\s:\[][^:\n]*:\s+`
const CATEGORY = String.raw`(?:\[[^\]\n,]*\])+\s*`

type LogRule = [RegExp, string[]]

function levelLines(levels: string, levelToken: string, messageToken = ''): LogRule[] {
  const level = String.raw`(?:${levels}):?\s+`
  return [
    [new RegExp(`^(${TIMESTAMP})(${level})(${SOURCE})(${CATEGORY})(.*)$`),
      ['log.timestamp', levelToken, 'log.source', 'log.category', messageToken]],
    [new RegExp(`^(${TIMESTAMP})(${level})(${SOURCE})(.*)$`),
      ['log.timestamp', levelToken, 'log.source', messageToken]],
    [new RegExp(`^(${TIMESTAMP})(${level})(${CATEGORY})(.*)$`),
      ['log.timestamp', levelToken, 'log.category', messageToken]],
    [new RegExp(`^(${TIMESTAMP})(${level})(.*)$`),
      ['log.timestamp', levelToken, messageToken]],
    [new RegExp(`^(${level})(${SOURCE})(${CATEGORY})(.*)$`),
      [levelToken, 'log.source', 'log.category', messageToken]],
    [new RegExp(`^(${level})(${SOURCE})(.*)$`),
      [levelToken, 'log.source', messageToken]],
    [new RegExp(`^(${level})(${CATEGORY})(.*)$`),
      [levelToken, 'log.category', messageToken]],
    [new RegExp(`^(${level})(.*)$`), [levelToken, messageToken]],
  ]
}

export function registerLogLanguage(m: Pick<typeof monaco, 'languages'>): void {
  if (registered) return
  registered = true

  m.languages.register({ id: LOG_LANGUAGE_ID })
  m.languages.setMonarchTokensProvider(LOG_LANGUAGE_ID, {
    tokenizer: {
      root: [
        [/^((?:>>>|\.\.\.))(\s?)/, ['log.input', '']],
        ...levelLines('CRITICAL|HACK', 'log.critical', 'log.critical'),
        ...levelLines('ERROR', 'log.error', 'log.error'),
        ...levelLines('WARNING', 'log.warning', 'log.warning'),
        ...levelLines('NOTICE|HOOK', 'log.notice'),
        ...levelLines('DEBUG|TRACE', 'log.debug'),
        ...levelLines('INFO', 'log.info'),
        [/^Traceback \(most recent call last\):.*$/, 'log.error'],
        [/^\s*File\s+"[^"]+",\s+line\s+\d+.*$/, 'log.traceback'],
        [/^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+:?\s+/, 'log.timestamp'],
        [/^[^\s:\[][^:\n]*:\s+/, 'log.source'],
        [/(?:\[[^\]\n,]*\])+/, 'log.category'],
        [/\b(?:CRITICAL|HACK)\b/, 'log.critical'],
        [/\bERROR\b/, 'log.error'],
        [/\bWARNING\b/, 'log.warning'],
        [/\b(?:NOTICE|HOOK)\b/, 'log.notice'],
        [/\b(?:DEBUG|TRACE)\b/, 'log.debug'],
        [/\bINFO\b/, 'log.info'],
      ],
    },
  })
}
