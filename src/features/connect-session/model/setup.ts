import { api, pickFile, pickFolder, type GameInfo } from '@/shared/api'
import { loadState, saveState } from '@/shared/lib'
import { consoleBus } from '@/entities/console'
import { connect, disconnect } from './connect'

export const LAST_GAME_KEY = 'lastGame'
export const RECENT_REPLAYS_KEY = 'recentReplays'

export function replayExtension(game: GameInfo): 'mtreplay' | 'wotreplay' {
  return game.exe.toLowerCase() === 'tanki.exe' ? 'mtreplay' : 'wotreplay'
}

export function recentReplays(): string[] {
  const saved = loadState<unknown>(RECENT_REPLAYS_KEY, [])
  return Array.isArray(saved) ? saved.filter((path): path is string => typeof path === 'string').slice(0, 5) : []
}

export async function pickReplay(game: GameInfo): Promise<string | null> {
  return pickFile('Select replay', replayExtension(game))
}

export async function detectGames(): Promise<GameInfo[]> {
  try {
    return await api.detectGames()
  } catch {
    return []
  }
}

/** Let the user pick a folder manually; validates it's a real WoT install. */
export async function pickGame(): Promise<GameInfo | null> {
  try {
    const dir = await pickFolder('Select your World of Tanks / Мир танков folder')
    if (!dir) return null
    const info = await api.inspectGameDir(dir)
    if (!info) {
      consoleBus.system(`not a WoT install (no Tanki.exe/WorldOfTanks.exe): ${dir}\n`)
      return null
    }
    saveState(LAST_GAME_KEY, info)
    return info
  } catch (error) {
    consoleBus.system(`folder pick failed: ${String(error)}\n`)
    return null
  }
}

/** PJOrion-style one click: install the agent, optionally launch, then connect. */
export async function setupAndConnect(game: GameInfo, launch: boolean, replay?: string): Promise<void> {
  try {
    saveState(LAST_GAME_KEY, game)
    consoleBus.system(`installing agent into ${game.path} (mods/${game.modsVersion})\n`)
    const buffer = await api.installAgent(game.path, game.modsVersion)
    consoleBus.system('agent installed\n')
    if (launch) {
      consoleBus.system(`launching ${replay ?? game.exe}\n`)
    }
    const connection = connect(buffer)
    if (launch) {
      try {
        await api.launchGame(game.path, game.exe, replay)
      } catch (error) {
        await disconnect()
        throw error
      }
    }
    await connection
  } catch (error) {
    consoleBus.system(`setup failed: ${String(error)}\n`)
  }
}
