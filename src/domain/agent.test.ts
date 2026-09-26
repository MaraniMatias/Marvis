import { describe, expect, it } from "vitest";
import { agentAttention, defaultAgentSession, isTurnEvent, sortAgentSessions } from "./agent";
import type { AgentSession } from "./agent";

function session(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id: "ses_one",
    checkoutId: "checkout:first",
    title: "review",
    busy: false,
    idleAt: 1,
    blockedOnPermission: false,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

describe("agent session helpers", () => {
  it("puts the most recently touched session first and uses it as the default target", () => {
    const sessions = [session({ id: "ses_old", updatedAt: 1 }), session({ id: "ses_new", updatedAt: 9 })];
    expect(sortAgentSessions(sessions).map((item) => item.id)).toEqual(["ses_new", "ses_old"]);
    expect(defaultAgentSession(sessions)?.id).toBe("ses_new");
    expect(defaultAgentSession([])).toBeNull();
  });

  it("breaks a tie deterministically so the default target never flickers", () => {
    const sessions = [session({ id: "ses_b", updatedAt: 5 }), session({ id: "ses_a", updatedAt: 5 })];
    expect(defaultAgentSession(sessions)?.id).toBe("ses_a");
  });

  it("reports a blocked session ahead of a busy one, and neither when absent", () => {
    expect(agentAttention(undefined)).toBe("none");
    expect(agentAttention(session())).toBe("none");
    expect(agentAttention(session({ busy: true }))).toBe("busy");
    expect(agentAttention(session({ busy: true, blockedOnPermission: true }))).toBe("blocked");
    expect(agentAttention(session({ blockedOnPermission: true }))).toBe("blocked");
  });

  it("treats only turn transitions as turn events", () => {
    expect(isTurnEvent("turnStarted")).toBe(true);
    expect(isTurnEvent("turnFinished")).toBe(true);
    expect(isTurnEvent("turnFailed")).toBe(true);
    expect(isTurnEvent("toolCalled")).toBe(false);
    expect(isTurnEvent("permissionAsked")).toBe(false);
    expect(isTurnEvent("unknown")).toBe(false);
  });
});
