import { useEffect, useRef, useState } from 'react'
import type { GameInfo } from '@/shared/api'
import { loadState, saveState } from '@/shared/lib'
import { useSession } from '@/entities/session'
import { connect, disconnect } from '../model/connect'
import {
  detectGames,
  pickGame,
  pickReplay,
  recentReplays,
  replayExtension,
  setupAndConnect,
  LAST_GAME_KEY,
  RECENT_REPLAYS_KEY,
} from '../model/setup'

const BTN =
  'h-7 rounded border border-edge px-3 text-[12px] text-fg transition-colors hover:border-live disabled:opacity-40'

function isGameInfo(value: unknown): value is GameInfo {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as GameInfo).path === 'string' &&
    typeof (value as GameInfo).version === 'string' &&
    typeof (value as GameInfo).modsVersion === 'string' &&
    typeof (value as GameInfo).exe === 'string'
  )
}

export function ConnectControls() {
  const status = useSession((s) => s.status)
  const [games, setGames] = useState<GameInfo[]>([])
  const [selected, setSelected] = useState(0)
  const [replays, setReplays] = useState(recentReplays)
  const gameMenu = useRef<HTMLDetailsElement>(null)
  const replayMenu = useRef<HTMLDetailsElement>(null)

  useEffect(() => {
    void detectGames().then((detected) => {
      const saved = loadState<unknown>(LAST_GAME_KEY, null)
      if (!isGameInfo(saved)) {
        setGames(detected)
        return
      }
      const at = detected.findIndex((g) => g.path.toLowerCase() === saved.path.toLowerCase())
      if (at >= 0) {
        setGames(detected)
        setSelected(at)
      } else {
        setGames([...detected, saved])
        setSelected(detected.length)
      }
    })
  }, [])

  useEffect(() => {
    const closeMenus = (event: PointerEvent) => {
      for (const menu of [gameMenu.current, replayMenu.current]) {
        if (menu?.open && !menu.contains(event.target as Node)) menu.open = false
      }
    }
    document.addEventListener('pointerdown', closeMenus)
    return () => document.removeEventListener('pointerdown', closeMenus)
  }, [])

  const connected = status === 'connected'
  const busy = status === 'connecting'
  const game = games[selected]

  const onBrowse = async () => {
    const info = await pickGame()
    if (!info) return
    setGames((prev) => {
      const at = prev.findIndex((g) => g.path.toLowerCase() === info.path.toLowerCase())
      if (at >= 0) {
        setSelected(at)
        return prev
      }
      setSelected(prev.length)
      return [...prev, info]
    })
  }

  const launchReplay = (path: string) => {
    if (!game) return
    const next = [path, ...replays.filter((item) => item.toLowerCase() !== path.toLowerCase())].slice(0, 5)
    setReplays(next)
    saveState(RECENT_REPLAYS_KEY, next)
    if (replayMenu.current) replayMenu.current.open = false
    void setupAndConnect(game, true, path)
  }

  const onPickReplay = async () => {
    if (!game) return
    const path = await pickReplay(game)
    if (path) launchReplay(path)
  }

  if (busy) {
    return (
      <button type="button" onClick={() => void disconnect()} className={BTN}>
        Cancel
      </button>
    )
  }

  if (connected) {
    return (
      <button type="button" onClick={() => void disconnect()} className={BTN}>
        Disconnect
      </button>
    )
  }

  return (
    <div className="flex items-center gap-2">
      {games.length > 0 ? (
        <details ref={gameMenu} className="relative z-30">
          <summary className="flex h-7 max-w-72 cursor-pointer list-none items-center gap-2 rounded border border-edge bg-elevated px-2 text-[12px] text-fg transition-colors hover:border-live [&::-webkit-details-marker]:hidden">
            <span className="min-w-0 truncate">
              {game?.path.split(/[\\/]/).pop()} · {game?.version}
              {game?.installed ? ' · installed' : ''}
            </span>
            <svg viewBox="0 0 16 16" fill="none" aria-hidden="true" className="h-3 w-3 shrink-0 stroke-current">
              <path d="m2.5 5 5.5 5.5L13.5 5" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </summary>
          <div className="absolute left-0 top-8 w-72 rounded border border-edge bg-elevated p-1 shadow-lg">
            <div className="px-2 py-1.5 text-[11px] font-medium text-muted">Select Game</div>
            {games.map((candidate, index) => (
              <button
                key={candidate.path}
                type="button"
                title={candidate.path}
                aria-current={index === selected}
                onClick={() => {
                  setSelected(index)
                  if (gameMenu.current) gameMenu.current.open = false
                }}
                className={`block w-full truncate rounded px-2 py-1.5 text-left text-[11px] hover:bg-panel ${index === selected ? 'bg-panel text-live' : 'text-fg'}`}
              >
                {candidate.path.split(/[\\/]/).pop()} · {candidate.version}
                {candidate.installed ? ' · installed' : ''}
              </button>
            ))}
          </div>
        </details>
      ) : (
        <span className="text-[12px] text-faint">no client detected</span>
      )}
      <button type="button" onClick={() => void onBrowse()} disabled={busy} className={BTN} title="Pick the game folder manually">
        Browse…
      </button>
      <div className="flex items-center [&:has(summary:hover)>button]:border-r-live">
        <button
          type="button"
          onClick={() => game && void setupAndConnect(game, true)}
          disabled={!game || busy}
          className={`${BTN} rounded-r-none border-live/40`}
        >
          {busy ? 'Waiting for game…' : 'Launch Game'}
        </button>
        <details ref={replayMenu} className="relative z-30">
          <summary
            title="Launch replay"
            aria-label="Launch replay"
            aria-disabled={!game || busy}
            tabIndex={!game || busy ? -1 : 0}
            className={`flex h-7 w-7 cursor-pointer list-none items-center justify-center rounded rounded-l-none border border-l-0 border-live/40 text-fg transition-colors hover:border-live [&::-webkit-details-marker]:hidden ${!game || busy ? 'pointer-events-none opacity-40' : ''}`}
          >
            <svg viewBox="0 0 16 16" fill="none" aria-hidden="true" className="h-3 w-3 mt-0.5 stroke-current">
              <path d="m2.5 5 5.5 5.5L13.5 5" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </summary>
          <div className="absolute right-0 top-8 w-72 rounded border border-edge bg-elevated p-1 shadow-lg">
            <div className="px-2 py-1.5 text-[11px] font-medium text-muted">Launch Replay</div>
            {game &&
              replays
                .filter((path) => path.toLowerCase().endsWith(`.${replayExtension(game)}`))
                .map((path) => (
                  <button
                    key={path}
                    type="button"
                    title={path}
                    onClick={() => launchReplay(path)}
                    className="block w-full truncate rounded px-2 py-1.5 text-left text-[11px] text-fg hover:bg-panel"
                  >
                    {path.split(/[\\/]/).pop()}
                  </button>
                ))}
            <button
              type="button"
              onClick={() => void onPickReplay()}
              className="mt-1 block w-full rounded border-t border-edge px-2 py-1.5 text-left text-[11px] text-live hover:bg-panel"
            >
              Choose…
            </button>
          </div>
        </details>
      </div>
      <button
        type="button"
        onClick={() => void connect()}
        disabled={busy}
        className={BTN}
        title="Connect to an already-running client"
      >
        Connect
      </button>
    </div>
  )
}
