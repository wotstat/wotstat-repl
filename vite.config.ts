import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { readFileSync } from 'node:fs'
import path from 'node:path'

const host = process.env.TAURI_DEV_HOST
const packageMetadata = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
) as { version: string }
const appVersion = process.env.WOTSTAT_VERSION?.trim() || packageMetadata.version

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(appVersion)) {
  throw new Error(`Invalid WOTSTAT_VERSION: ${appVersion}`)
}

export default defineConfig(({ mode }) => ({
  define: {
    __APP_VERSION__: JSON.stringify(appVersion),
  },
  plugins: [
    react(),
    tailwindcss(),
    {
      name: 'repl-runtime-entry',
      transformIndexHtml: {
        order: 'pre',
        handler(html) {
          return mode === 'web'
            ? html.replace('/src/app/main.tsx', '/src/app/main.web.tsx')
            : html
        },
      },
    },
  ],
  build: {
    // The UI is bundled into the desktop app and never crosses a network.
    chunkSizeWarningLimit: 6_000,
    rolldownOptions: {
      output: { codeSplitting: false },
    },
  },
  resolve: {
    alias: { '@': path.resolve(import.meta.dirname, 'src') },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host ?? false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
  },
}))
