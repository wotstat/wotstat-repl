import { describe, expect, test } from "bun:test";
import { agentLanPresentation } from "./agentNetworkState";

describe("Agent LAN presentation", () => {
  test("is gray and hides listener while LAN is disabled", () => {
    expect(agentLanPresentation(false, true)).toEqual({
      indicatorClass: "bg-faint",
      label: "Agent LAN",
      showListener: false,
    });
  });

  test("is orange while LAN is enabled without security", () => {
    expect(agentLanPresentation(true, false).indicatorClass).toBe(
      "bg-warn",
    );
  });

  test("is green while LAN and security are enabled", () => {
    expect(
      agentLanPresentation(true, true).indicatorClass,
    ).toBe("bg-live");
  });
});
