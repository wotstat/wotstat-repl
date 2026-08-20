export interface AgentLanPresentation {
  indicatorClass: string;
  label: string;
  showListener: boolean;
}

export function agentLanPresentation(
  lanEnabled: boolean,
  secureEnabled: boolean,
): AgentLanPresentation {
  if (!lanEnabled) {
    return {
      indicatorClass: "bg-faint",
      label: "Agent LAN",
      showListener: false,
    };
  }

  return {
    indicatorClass: secureEnabled ? "bg-ok" : "bg-warn",
    label: "Agent LAN",
    showListener: true,
  };
}
