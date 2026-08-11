import { open } from '@tauri-apps/plugin-dialog'

export async function pickFolder(title: string): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, title })
  return typeof picked === 'string' ? picked : null
}

export async function pickFile(title: string, extension: string): Promise<string | null> {
  const picked = await open({
    directory: false,
    multiple: false,
    title,
    filters: [{ name: 'Replay', extensions: [extension] }],
  })
  return typeof picked === 'string' ? picked : null
}
