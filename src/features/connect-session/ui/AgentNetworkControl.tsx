import { useEffect, useRef, useState } from "react";
import { api, type AgentConnectionInfo } from "@/shared/api";
import {
  AGENT_LAN_STORAGE_KEY,
  AGENT_SECURE_STORAGE_KEY,
} from "@/shared/config";
import { loadState, saveState } from "@/shared/lib";
import { useSession } from "@/entities/session";

export function AgentNetworkControl() {
  const root = useRef<HTMLDivElement>(null);
  const status = useSession((state) => state.status);
  const [open, setOpen] = useState(false);
  const [lanEnabled, setLanEnabled] = useState(() =>
    loadState(AGENT_LAN_STORAGE_KEY, false),
  );
  const [secureEnabled, setSecureEnabled] = useState(() =>
    loadState(AGENT_SECURE_STORAGE_KEY, true),
  );
  const [info, setInfo] = useState<AgentConnectionInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!open) return;
    void api
      .agentConnectionInfo()
      .then((value) => {
        setInfo(value);
        setError(null);
      })
      .catch((reason: unknown) => setError(String(reason)));
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);

  const toggleLan = (enabled: boolean) => {
    setLanEnabled(enabled);
    saveState(AGENT_LAN_STORAGE_KEY, enabled);
  };

  const toggleSecure = (enabled: boolean) => {
    setSecureEnabled(enabled);
    saveState(AGENT_SECURE_STORAGE_KEY, enabled);
  };

  const indicatorColor =
    status === "connected" ? "bg-live" : lanEnabled ? "bg-warn" : "bg-faint";

  const copyConfig = async () => {
    if (!info) return;
    try {
      await navigator.clipboard.writeText(info.clientConfig);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      setError("Could not copy agent config");
    }
  };

  return (
    <div ref={root} className="relative">
      <button
        type="button"
        title={`Agent network: ${lanEnabled ? "LAN" : "localhost"}, ${secureEnabled ? "secure" : "no token required"}`}
        aria-expanded={open}
        aria-controls="agent-network-popover"
        onClick={() => setOpen((value) => !value)}
        className="inline-flex h-5 items-center gap-1.5 rounded px-1.5 text-[11px] text-muted transition-colors hover:bg-elevated hover:text-fg"
      >
        <span className={`size-1.5 rounded-full ${indicatorColor}`} />
        Agent {lanEnabled ? "LAN" : "local"}
      </button>

      {open && (
        <section
          id="agent-network-popover"
          role="dialog"
          aria-label="Agent network settings"
          className="absolute bottom-7 right-0 z-40 w-96 select-text rounded border border-edge bg-elevated p-3 text-left shadow-2xl"
        >
          <div className="mb-3 text-[12px] font-medium text-fg">
            Agent network
          </div>
          <label className="flex items-center justify-between text-[11px] text-fg">
            Accept LAN connections
            <input
              type="checkbox"
              checked={lanEnabled}
              disabled={status !== "disconnected"}
              onChange={(event) => toggleLan(event.target.checked)}
              className="accent-live"
            />
          </label>
          <p className="mt-1 text-[10px] text-faint">
            LAN mode listens on all interfaces and enables UDP discovery. It
            applies on the next connection.
          </p>

          <label className="mt-3 flex items-center justify-between text-[11px] text-fg">
            Secure connection
            <input
              type="checkbox"
              checked={secureEnabled}
              disabled={status !== "disconnected"}
              onChange={(event) => toggleSecure(event.target.checked)}
              className="accent-live"
            />
          </label>
          <p
            className={`mt-1 text-[10px] ${secureEnabled ? "text-faint" : "text-warn"}`}
          >
            {secureEnabled
              ? "Requires the shared token from agent-network.json."
              : "No config is required. Any reachable agent may connect and use the REPL."}
          </p>

          <div className="mt-3 space-y-1">
            <span className="text-[10px] font-medium uppercase tracking-wide text-faint">
              Listener
            </span>
            <code className="block rounded bg-panel p-2 font-mono text-[10px] text-fg">
              {info
                ? lanEnabled
                  ? info.networkAddress
                  : info.localAddress
                : "Loading…"}
            </code>
          </div>

          {secureEnabled && (
            <div className="mt-3 space-y-1">
              <span className="text-[10px] font-medium uppercase tracking-wide text-faint">
                Remote game config
              </span>
              <code className="block max-h-32 overflow-auto whitespace-pre rounded bg-panel p-2 font-mono text-[10px] text-fg">
                {info?.clientConfig ?? "Loading…"}
              </code>
              <button
                type="button"
                disabled={!info}
                onClick={() => void copyConfig()}
                className="h-6 rounded border border-edge px-2 text-[10px] text-muted hover:border-live hover:text-fg disabled:opacity-40"
              >
                {copied ? "Copied" : "Copy config"}
              </button>
            </div>
          )}

          {secureEnabled && info && (
            <p className="mt-2 break-all text-[10px] text-faint">
              Place this file at{" "}
              <code>mods/configs/wotstat-repl/agent-network.json</code> under
              the game root. Local installs write it automatically. UI copy:{" "}
              {info.configPath}.
            </p>
          )}
          {error && (
            <p role="alert" className="mt-2 text-[10px] text-error">
              {error}
            </p>
          )}
        </section>
      )}
    </div>
  );
}
