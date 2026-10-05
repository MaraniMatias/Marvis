// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { computed, ref } from "vue";
import type { AgentEvent, AgentSession } from "../domain/agent";
import type { Checkout, Repo } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  listAgentSessions: vi.fn(),
  listAgentAgents: vi.fn(),
  createAgentSession: vi.fn(),
  stopAgent: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("../lib/ipc", () => ({
  listAgentSessions: mocks.listAgentSessions,
  listAgentAgents: mocks.listAgentAgents,
  createAgentSession: mocks.createAgentSession,
  stopAgent: mocks.stopAgent,
}));

import { applyAgentEvent, useAgentSessions } from "./agent-sessions";

function session(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    id: "ses_one",
    checkoutId: "checkout:first",
    title: "review",
    busy: false,
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

const checkout: Checkout = {
  id: "checkout:first",
  repoId: "repo:test",
  path: "/first",
  canonicalPath: "/first",
  isPrimary: true,
  changedFiles: 1,
  isMissing: false,
  sessions: [],
};
const gitRepo: Repo = {
  id: "repo:test",
  kind: "git",
  name: "test",
  root: "/first",
  defaultBranch: "trunk",
  checkouts: [],
  createdAt: "now",
  lastOpenedAt: "now",
};

async function settle() {
  for (let index = 0; index < 4; index += 1) await Promise.resolve();
}

describe("applyAgentEvent", () => {
  it("marks a started turn busy and a failed turn not busy", () => {
    const started = applyAgentEvent([session()], event({ kind: "turnStarted" }));
    expect(started[0].busy).toBe(true);
    const failed = applyAgentEvent(started, event({ kind: "turnFailed" }));
    expect(failed[0].busy).toBe(false);
  });

  it("reports an unanswerable permission instead of pretending the agent is working", () => {
    const blocked = applyAgentEvent([session({ busy: true })], event({ kind: "permissionAsked" }));
    expect(blocked[0].blockedOnPermission).toBe(true);
    expect(blocked[0].busy).toBe(false);
  });

  it("ignores events for another session and events with no session", () => {
    const sessions = [session()];
    expect(applyAgentEvent(sessions, event({ kind: "turnStarted", sessionId: "ses_other" }))).toEqual(sessions);
    expect(applyAgentEvent(sessions, event({ kind: "turnStarted", sessionId: null }))).toEqual(sessions);
  });
});

function event(overrides: Partial<AgentEvent>): AgentEvent {
  return {
    checkoutId: "checkout:first",
    sessionId: "ses_one",
    kind: "unknown",
    rawType: "session.inbox.enqueued",
    data: {},
    ...overrides,
  };
}

describe("useAgentSessions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listAgentSessions.mockResolvedValue([]);
    // An empty catalog is the common case: a row with no color of its own falls back.
    mocks.listAgentAgents.mockResolvedValue([]);
  });

  it("defaults the target to the most recently updated session", async () => {
    mocks.listAgentSessions.mockResolvedValue([
      session({ id: "ses_old", updatedAt: 1 }),
      session({ id: "ses_new", updatedAt: 9 }),
    ]);
    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await settle();

    expect(state.state).toBe("ready");
    expect(state.targetId).toBe("ses_new");
  });

  it("keeps a chosen target across a reload while it still exists", async () => {
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one" }), session({ id: "ses_two", updatedAt: 9 })]);
    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await settle();

    state.selectTarget("ses_one");
    expect(state.targetId).toBe("ses_one");

    expect(await state.reload()).toBe(true);
    expect(mocks.listAgentSessions).toHaveBeenCalledTimes(2);
    expect(state.targetId).toBe("ses_one");

    // Gone from the server: the target follows what is actually there.
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_two", updatedAt: 9 })]);
    expect(await state.reload()).toBe(true);
    expect(state.targetId).toBe("ses_two");
  });

  it("surfaces an agent that cannot start", async () => {
    mocks.listAgentSessions.mockRejectedValue({
      code: "agent_unavailable",
      message: "OpenCode is unavailable",
    });
    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await settle();

    expect(state.state).toBe("error");
    expect(state.error).toBe("OpenCode is unavailable");
    expect(state.sessions).toEqual([]);
  });

  it("does not start a server for a plain folder", async () => {
    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => ({ ...gitRepo, kind: "plain" }) as Repo),
    );
    await settle();

    expect(mocks.listAgentSessions).not.toHaveBeenCalled();
    expect(state.state).toBe("ready");
  });

  it("applies live events for the active checkout only", async () => {
    mocks.listAgentSessions.mockResolvedValue([session()]);
    let handler: ((event: { payload: AgentEvent }) => void) | undefined;
    mocks.listen.mockImplementation(async (_name: string, callback: typeof handler) => {
      handler = callback;
      return vi.fn();
    });
    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await vi.waitFor(() => expect(handler).toBeDefined());
    expect(mocks.listen).toHaveBeenCalledTimes(1);

    // `busy` is re-derived from the server after a turn event, so the durable record of the
    // event is what this asserts; the working state is covered by the completion tests.
    handler?.({ payload: event({ kind: "turnStarted" }) });
    await settle();
    expect(state.events.at(-1)?.kind).toBe("turnStarted");

    // An unanswerable permission is the one state the server cannot report for us.
    handler?.({ payload: event({ kind: "permissionAsked", rawType: "permission.asked" }) });
    await settle();
    expect(state.sessions[0].blockedOnPermission).toBe(true);
    expect(state.sessions[0].busy).toBe(false);

    handler?.({ payload: event({ kind: "turnStarted", checkoutId: "checkout:other" }) });
    await settle();
    // Another checkout's events are never applied here.
    expect(state.events.every((item) => item.checkoutId === "checkout:first")).toBe(true);
  });

  it("counts a turn that was seen to finish, and does not invent one that never ran", async () => {
    vi.useFakeTimers();
    // A session that has never run has no idle time either, so it must not read as working.
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", idleAt: null })]);
    let handler: ((event: { payload: AgentEvent }) => void) | undefined;
    mocks.listen.mockImplementation(async (_name: string, callback: typeof handler) => {
      handler = callback;
      return vi.fn();
    });
    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await settle();
    expect(state.sessions[0].busy).toBe(false);
    expect(state.turnsCompleted).toBe(0);

    // Now a turn is seen starting, and only then is the session working.
    handler?.({ payload: event({ kind: "turnStarted" }) });
    await settle();
    expect(state.sessions[0].busy).toBe(true);

    // The server reports the session idle: that transition is the completion signal.
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", idleAt: 99 })]);
    await vi.advanceTimersByTimeAsync(2000);
    await settle();
    expect(state.turnsCompleted).toBe(1);
    expect(state.sessions[0].busy).toBe(false);

    // Nothing is running any more, so an idle checkout is not polled.
    const callsWhenIdle = mocks.listAgentSessions.mock.calls.length;
    await vi.advanceTimersByTimeAsync(6000);
    expect(mocks.listAgentSessions.mock.calls.length).toBe(callsWhenIdle);
    expect(state.turnsCompleted).toBe(1);
    vi.useRealTimers();
  });

  it("keeps counting when a turn fails, without waiting for an idle time", async () => {
    vi.useFakeTimers();
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", idleAt: null })]);
    let handler: ((event: { payload: AgentEvent }) => void) | undefined;
    mocks.listen.mockImplementation(async (_name: string, callback: typeof handler) => {
      handler = callback;
      return vi.fn();
    });
    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await settle();

    handler?.({ payload: event({ kind: "turnStarted" }) });
    await settle();
    // A failed turn reports an error rather than going idle, and still ends.
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", idleAt: 99 })]);
    handler?.({ payload: event({ kind: "turnFailed" }) });
    await settle();
    await vi.advanceTimersByTimeAsync(2000);
    await settle();
    expect(state.turnsCompleted).toBe(1);
    vi.useRealTimers();
  });

  it("clears its sessions when the checkout changes", async () => {
    mocks.listAgentSessions.mockResolvedValue([session()]);
    const checkoutRef = ref<Checkout | null>(checkout);
    const state = useAgentSessions(
      computed(() => checkoutRef.value),
      computed(() => gitRepo),
    );
    await settle();
    expect(state.sessions).toHaveLength(1);

    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_second", checkoutId: "checkout:second" })]);
    checkoutRef.value = { ...checkout, id: "checkout:second" };
    await settle();

    expect(state.checkoutId).toBe("checkout:second");
    expect(state.sessions.map((item) => item.id)).toEqual(["ses_second"]);
  });

  it("re-reads a catalog that is still filling in, and stops asking once it is whole", async () => {
    // What the server really does: it hands over its built-in agents first and the user's own
    // agents a moment later. A catalog read once per checkout would leave that row without its
    // colour for the rest of the session.
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_coder", agent: "coder" })]);
    mocks.listAgentAgents.mockResolvedValue([
      { id: "build", name: "Build", mode: "primary", color: null, hidden: false },
    ]);

    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await settle();

    // The first read is behind, and the row is drawn in the accent it falls back to.
    expect(state.headline).toEqual({ label: "coder", color: null, attention: "none" });

    // The agent turns up in the catalog, so it is read again and the row follows it.
    mocks.listAgentAgents.mockResolvedValue([
      { id: "build", name: "Build", mode: "primary", color: null, hidden: false },
      { id: "coder", name: "coder", mode: "all", color: "#4ed6bf", hidden: false },
    ]);
    mocks.listAgentAgents.mockClear();
    await state.reload();
    expect(mocks.listAgentAgents).toHaveBeenCalledTimes(1);
    expect(state.headline).toEqual({ label: "coder", color: "#4ed6bf", attention: "none" });

    // Whole now: a reload that changes nothing does not ask again.
    await state.reload();
    expect(mocks.listAgentAgents).toHaveBeenCalledTimes(1);
  });

  it("asks again while there is no service, and connects when one appears", async () => {
    vi.useFakeTimers();
    mocks.listAgentSessions.mockRejectedValue(new Error("OpenCode is not running."));

    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await vi.advanceTimersByTimeAsync(0);
    expect(state.state).toBe("error");
    const whileDisconnected = mocks.listAgentSessions.mock.calls.length;
    expect(whileDisconnected).toBeGreaterThan(0);

    // Nothing is started on Marvis's side, so the only way to notice a service is to ask again.
    await vi.advanceTimersByTimeAsync(5000);
    expect(mocks.listAgentSessions.mock.calls.length).toBeGreaterThan(whileDisconnected);

    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", title: "later" })]);
    await vi.advanceTimersByTimeAsync(5000);

    expect(state.state).toBe("ready");
    expect(state.sessions.map((entry) => entry.title)).toEqual(["later"]);

    // Connected, it stops asking about a service that is not missing.
    const onceConnected = mocks.listAgentSessions.mock.calls.length;
    await vi.advanceTimersByTimeAsync(10000);
    expect(mocks.listAgentSessions.mock.calls.length).toBe(onceConnected);
  });

  it("keeps a row on the accent when the catalog cannot be read at all", async () => {
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", agent: "plan" })]);
    mocks.listAgentAgents.mockRejectedValue(new Error("no such route"));

    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await settle();
    await state.reload();

    // A catalog that cannot be read is not an error: the session is still listed and named.
    expect(state.state).toBe("ready");
    expect(state.headline).toEqual({ label: "plan", color: null, attention: "none" });
  });
});
