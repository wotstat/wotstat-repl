import { describe, expect, test } from 'bun:test'
import { LOG_LANGUAGE_ID, registerLogLanguage } from './logLanguage'

describe('log language', () => {
  test('never leaves an unmatched capture group when timestamp is hidden', () => {
    let provider: { tokenizer: { root: [RegExp, unknown][] } } | undefined
    registerLogLanguage({
      languages: {
        register(language: { id: string }) {
          expect(language.id).toBe(LOG_LANGUAGE_ID)
        },
        setMonarchTokensProvider(_id: string, value: unknown) {
          provider = value as typeof provider
          return { dispose() {} }
        },
      },
    } as never)

    const lines = [
      'DEBUG: [Gameface] View successfully loaded mono/hangar/header',
      'DEBUG: Main: [Gameface] View successfully loaded mono/hangar/header',
      '2026-08-20 05:17:07.011: INFO: Renou_EU',
      '2026-08-20 05:17:07.011: INFO: Main: Renou_EU',
    ]

    for (const line of lines) {
      const rule = provider?.tokenizer.root.find(([pattern]) => pattern.test(line))
      expect(rule).toBeDefined()
      const match = rule?.[0].exec(line)
      expect(match).not.toBeNull()
      expect(match?.slice(1).every((group) => group !== undefined)).toBe(true)
    }
  })
})
