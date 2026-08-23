import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { AppLayout } from '@/app/layout'
import { StudioPage } from '@/pages/studio'
import { APP_TITLE } from '@/shared/config'
import { configureReplRuntime, tauriReplRuntime } from '@/shared/repl'
import './styles/index.css'

configureReplRuntime(tauriReplRuntime)
document.title = APP_TITLE

const rootEl = document.getElementById('root')
if (!rootEl) throw new Error('root element not found')

createRoot(rootEl).render(
  <StrictMode>
    <AppLayout>
      <StudioPage />
    </AppLayout>
  </StrictMode>,
)
