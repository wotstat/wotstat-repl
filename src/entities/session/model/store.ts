import { create } from 'zustand'

export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected'

interface SessionState {
  status: ConnectionStatus
  endpoint: string
  agentVersion: string | null
  agentPid: number | null
  setStatus: (status: ConnectionStatus) => void
  setEndpoint: (endpoint: string) => void
  setHello: (hello: { version?: string | null; pid?: number | null }) => void
}

export const useSession = create<SessionState>((set) => ({
  status: 'disconnected',
  endpoint: '',
  agentVersion: null,
  agentPid: null,
  setStatus: (status) => set({ status }),
  setEndpoint: (endpoint) => set({ endpoint }),
  setHello: ({ version, pid }) =>
    set({ agentVersion: version ?? null, agentPid: pid ?? null }),
}))
