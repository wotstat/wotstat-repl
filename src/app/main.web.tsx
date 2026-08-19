import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { AppLayout } from '@/app/layout'
import { EmbeddedStudioPage } from '@/pages/studio'
import { configureReplRuntime, webReplRuntime } from '@/shared/repl'
import './styles/index.css'

configureReplRuntime(webReplRuntime)

const rootEl = document.getElementById('root')
if (!rootEl) throw new Error('root element not found')

createRoot(rootEl).render(
  <StrictMode>
    <AppLayout>
      <EmbeddedStudioPage />
    </AppLayout>
  </StrictMode>,
)
