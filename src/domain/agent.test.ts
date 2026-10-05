import { describe, expect, it } from "vitest";
import {
  agentAttention,
  agentColor,
  agentLabel,
  defaultAgentSession,
  headlineSession,
  isTurnEvent,
  sortAgentSessions,
} from "./agent";
import type { AgentSession } from "./agent";

function session(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id: "ses_one",
    checkoutId: "checkout:first",
    title: "review",
    running: false,
    idleAt: 1,
    blockedOnPermission: false,
    agent: null,
    model: null,
    parentId: null,
    outcome: null,
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
    expect(agentAttention(session({ running: true }))).toBe("busy");
    expect(agentAttention(session({ running: true, blockedOnPermission: true }))).toBe("blocked");
    expect(agentAttention(session({ blockedOnPermission: true }))).toBe("blocked");
  });

  it("reads a turn the server ended badly as the last word, not as quiet", () => {
    expect(agentAttention(session({ outcome: "failed" }))).toBe("failed");
    expect(agentAttention(session({ outcome: "interrupted" }))).toBe("none");
    expect(agentAttention(session({ outcome: "succeeded" }))).toBe("none");
    // Working right now outranks a failure that already happened.
    expect(agentAttention(session({ running: true, outcome: "failed" }))).toBe("busy");
  });

  it("takes the agent's color only when it is a hex a stylesheet can use", () => {
    const agents = [
      { id: "plan", name: "Plan", mode: "primary", color: "#FF966C", hidden: false },
      { id: "coder", name: "coder", mode: "all", color: "#4ed6bf", hidden: false },
      { id: "build", name: "Build", mode: "primary", color: null, hidden: false },
      // The value is the server's and is about to become a `background`, so it is checked.
      { id: "hostile", name: "Hostile", mode: "primary", color: "red; background: url(x)", hidden: false },
    ];
    expect(agentColor(agents, "plan")).toBe("#FF966C");
    expect(agentColor(agents, "coder")).toBe("#4ed6bf");
    expect(agentColor(agents, "build")).toBeNull();
    expect(agentColor(agents, "hostile")).toBeNull();
    expect(agentColor(agents, "absent")).toBeNull();
    expect(agentColor(agents, null)).toBeNull();
  });

  it("names an agent by its display name, falling back to its id", () => {
    const agents = [
      { id: "plan", name: "Plan", mode: "primary", color: null, hidden: false },
      { id: "bare", name: "", mode: "primary", color: null, hidden: false },
    ];
    expect(agentLabel(agents, "plan")).toBe("Plan");
    expect(agentLabel(agents, "bare")).toBe("bare");
    // An agent the catalog has not caught up with still answers to its id, which is its name.
    expect(agentLabel(agents, "absent")).toBe("absent");
    // No agent at all is the one case with nothing to say.
    expect(agentLabel(agents, null)).toBeNull();
  });

  it("names the loudest session for a row, and the round's target when nothing is loud", () => {
    const quiet = [session({ id: "ses_quiet", updatedAt: 9, agent: "plan" })];
    const busy = session({ id: "ses_busy", updatedAt: 1, running: true, agent: "coder" });

    // Attention outranks recency: the row's news is the busy turn, not the newest idle one.
    expect(headlineSession([...quiet, busy], null)?.id).toBe("ses_busy");
    // With nothing to report it agrees with where a review would go.
    expect(headlineSession(quiet, "ses_quiet")?.id).toBe("ses_quiet");
    expect(headlineSession(quiet, "ses_gone")?.id).toBe("ses_quiet");
    expect(headlineSession([], "ses_quiet")).toBeNull();
  });

  it("skips a session that never ran, which has no agent to name", () => {
    // A fresh session is the most recently touched and has no agent yet, so it would blank a
    // row that has a real agent to show.
    const neverRan = session({ id: "ses_fresh", updatedAt: 99, agent: null });
    const real = session({ id: "ses_real", updatedAt: 1, agent: "coder" });

    expect(headlineSession([neverRan, real], "ses_fresh")?.id).toBe("ses_real");
    expect(headlineSession([neverRan, real], "ses_real")?.id).toBe("ses_real");
    // With nothing to name there is no row to draw, rather than a nameless one.
    expect(headlineSession([neverRan], "ses_fresh")).toBeNull();
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
