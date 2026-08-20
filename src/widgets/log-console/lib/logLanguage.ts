import type * as monaco from 'monaco-editor'

export const LOG_LANGUAGE_ID = 'wms-log'

let registered = false

const TIMESTAMP = String.raw`\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+:?\s+`
const CATEGORY = String.raw`(?:\[[^\]\n,]*\])+\s*`

function levelLine(levels: string): RegExp {
  return new RegExp(`^(${TIMESTAMP})?((?:${levels}):?\\s+)(${CATEGORY})?(.*)$`)
}

export function registerLogLanguage(m: Pick<typeof monaco, 'languages'>): void {
  if (registered) return
  registered = true

  m.languages.register({ id: LOG_LANGUAGE_ID })
  m.languages.setMonarchTokensProvider(LOG_LANGUAGE_ID, {
    tokenizer: {
      root: [
        [/^((?:>>>|\.\.\.))(\s?)/, ['log.input', '']],
        [levelLine('CRITICAL|HACK'), ['log.timestamp', 'log.critical', 'log.category', 'log.critical']],
        [levelLine('ERROR'), ['log.timestamp', 'log.error', 'log.category', 'log.error']],
        [levelLine('WARNING'), ['log.timestamp', 'log.warning', 'log.category', 'log.warning']],
        [levelLine('NOTICE|HOOK'), ['log.timestamp', 'log.notice', 'log.category', '']],
        [levelLine('DEBUG|TRACE'), ['log.timestamp', 'log.debug', 'log.category', '']],
        [levelLine('INFO'), ['log.timestamp', 'log.info', 'log.category', '']],
        [/^Traceback \(most recent call last\):.*$/, 'log.error'],
        [/^\s*File\s+"[^"]+",\s+line\s+\d+.*$/, 'log.traceback'],
        [/^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+:?\s+/, 'log.timestamp'],
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
