import { useEffect } from 'react'
import { connect, disconnect } from '@/features/connect-session'
import { CommandPalette } from '@/widgets/command-palette'
import { StatusBar } from '@/widgets/status-bar'
import { AppHeader } from './AppHeader'
import { ReplWorkspace } from './ReplWorkspace'

export function EmbeddedStudioPage() {
  useEffect(() => {
    void connect()
    return () => {
      void disconnect()
    }
  }, [])

  return (
    <>
      <AppHeader />
      <ReplWorkspace />
      <StatusBar />
      <CommandPalette />
    </>
  )
}
