import packageMetadata from '../../../package.json'

export const APP_NAME = 'WotStat REPL'
export const APP_VERSION =
  typeof __APP_VERSION__ === 'string' ? __APP_VERSION__ : packageMetadata.version
export const APP_TITLE = `${APP_NAME} v${APP_VERSION}`
export const DEV_SERVER_PORT = 1420;
export const COMPLETION_BUDGET_STORAGE_KEY = "completion.budget";
export const DEFAULT_COMPLETION_BUDGET = 120;
export const MAX_COMPLETION_BUDGET = 10000;
export const AGENT_LAN_STORAGE_KEY = "agent.lanEnabled";
export const AGENT_SECURE_STORAGE_KEY = "agent.secureEnabled";
