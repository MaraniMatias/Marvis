// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { computed, ref } from "vue";
import type { AgentEvent, AgentSession } from "../domain/agent";
import type { Checkout, Repo } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  listAgentSessions: vi.fn(),
  listAgentCandidateSessions: vi.fn(),
  listAgentAgents: vi.fn(),
  createAgentSession: vi.fn(),
  stopAgent: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("../lib/ipc", () => ({
  listAgentSessions: mocks.listAgentSessions,
  listAgentCandidateSessions: mocks.listAgentCandidateSessions,
  listAgentAgents: mocks.listAgentAgents,
  createAgentSession: mocks.createAgentSession,
  stopAgent: mocks.stopAgent,
}));

import { applyAgentEvent, useAgentSessions, useTerminalAgentRows } from "./agent-sessions";

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
  // Enough microtask turns for the two sequential reads a row makes (sessions, then the
  // catalog) plus the awaits in between them.
  for (let index = 0; index < 12; index += 1) await Promise.resolve();
}

describe("applyAgentEvent", () => {
  it("shows a started turn as working at once instead of waiting for the next read", () => {
    const started = applyAgentEvent([session()], event({ kind: "turnStarted" }));
    expect(started[0].running).toBe(true);
  });

  it("reports an unanswerable permission instead of pretending the agent is working", () => {
    const blocked = applyAgentEvent([session({ running: true })], event({ kind: "permissionAsked" }));
    expect(blocked[0].blockedOnPermission).toBe(true);
    expect(blocked[0].running).toBe(false);
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

    // `running` is re-read from the service after a turn event, so the durable record of the
    // event is what this asserts; the working state is covered by the completion tests.
    handler?.({ payload: event({ kind: "turnStarted" }) });
    await settle();
    expect(state.events.at(-1)?.kind).toBe("turnStarted");

    // An unanswerable permission is the one state the server cannot report for us.
    handler?.({ payload: event({ kind: "permissionAsked", rawType: "permission.asked" }) });
    await settle();
    expect(state.sessions[0].blockedOnPermission).toBe(true);
    expect(state.sessions[0].running).toBe(false);

    handler?.({ payload: event({ kind: "turnStarted", checkoutId: "checkout:other" }) });
    await settle();
    // Another checkout's events are never applied here.
    expect(state.events.every((item) => item.checkoutId === "checkout:first")).toBe(true);
  });

  it("counts a turn that was seen to finish, and does not invent one that never ran", async () => {
    vi.useFakeTimers();
    // A session that has never run is not reported running, so it must not read as working.
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
    expect(state.sessions[0].running).toBe(false);
    expect(state.turnsCompleted).toBe(0);

    // A turn starts, which the service announces and then reports. `running` is what it says,
    // not an inference from an idle time, so the row is working on the service's word. The
    // mocked read is set first, because the event triggers the read.
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", running: true })]);
    handler?.({ payload: event({ kind: "turnStarted" }) });
    await settle();
    expect(state.sessions[0].running).toBe(true);

    // The service stops reporting it: that transition is the completion signal.
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", idleAt: 99 })]);
    await vi.advanceTimersByTimeAsync(2000);
    await settle();
    expect(state.turnsCompleted).toBe(1);
    expect(state.sessions[0].running).toBe(false);

    // Nothing is running any more, so an idle checkout is not polled.
    const callsWhenIdle = mocks.listAgentSessions.mock.calls.length;
    await vi.advanceTimersByTimeAsync(6000);
    expect(mocks.listAgentSessions.mock.calls.length).toBe(callsWhenIdle);
    expect(state.turnsCompleted).toBe(1);
    vi.useRealTimers();
  });

  it("counts a turn the person started in their own TUI, which this client never prompted", async () => {
    vi.useFakeTimers();
    // No event on this stream is needed: the service reports the turn running on its own, which
    // is the whole point of reading `/api/session/active`. Before, a row could only call a turn
    // working if this client had seen it start, so a turn typed in the TUI looked idle.
    mocks.listen.mockImplementation(async () => vi.fn());
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", running: true })]);
    const state = useAgentSessions(
      computed(() => checkout),
      computed(() => gitRepo),
    );
    await vi.advanceTimersByTimeAsync(0);
    expect(state.sessions[0].running).toBe(true);
    expect(state.turnsCompleted).toBe(0);

    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", idleAt: 99 })]);
    await vi.advanceTimersByTimeAsync(2000);
    await settle();
    expect(state.turnsCompleted).toBe(1);
    vi.useRealTimers();
  });

  it("keeps counting when a turn fails, without waiting for an idle time", async () => {
    vi.useFakeTimers();
    mocks.listAgentSessions.mockResolvedValue([session({ id: "ses_one", running: true })]);
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

describe("useTerminalAgentRows", () => {
  /** Answers each checkout with the candidates its own service knows. */
  function perCheckout(byCheckout: Record<string, Partial<AgentSession>[]>): void {
    mocks.listAgentCandidateSessions.mockImplementation(async (checkoutId: string) =>
      (byCheckout[checkoutId] ?? []).map((overrides) => session({ checkoutId, ...overrides })),
    );
  }

  beforeEach(() => {
    mocks.listen.mockImplementation(async () => vi.fn());
    mocks.listAgentAgents.mockResolvedValue([
      { id: "build", name: "Build", mode: "primary", color: null, hidden: false },
      { id: "coder", name: "Coder", mode: "all", color: "#4ed6bf", hidden: false },
      { id: "plan", name: "Plan", mode: "primary", color: null, hidden: false },
    ]);
  });

  it("gives each terminal its own worktree's agent, independently", async () => {
    // The case this exists for: two Marvis terminals, two worktrees, two different agents. A
    // single active-checkout answer would paint both rows with whichever worktree is selected.
    perCheckout({
      "checkout:first": [{ id: "ses_one", agent: "coder", running: true }],
      "checkout:second": [{ id: "ses_two", agent: "plan" }],
    });

    const state = useTerminalAgentRows(computed(() => ["checkout:first", "checkout:second"]));
    await settle();

    // Every session is offered rather than one headline per checkout: a terminal row is about one
    // terminal, and the only thing that can say which session it has open is its own title. A row
    // matches against this list itself, so anything the list drops cannot be named by anybody.
    expect(state.byCheckout["checkout:first"].sessions).toEqual([
      { title: "review", agent: { label: "Coder", color: "#4ed6bf", attention: "busy" }, running: true, updatedAt: 1 },
    ]);
    expect(state.byCheckout["checkout:second"].sessions).toEqual([
      { title: "review", agent: { label: "Plan", color: null, attention: "none" }, running: false, updatedAt: 1 },
    ]);
  });

  it("keeps two terminals in one worktree from being told they are different sessions", async () => {
    // One checkout, one session. Both rows speak for the same directory scope because that is
    // the only scope OpenCode can answer in; what must not happen is either row claiming a
    // session id the service never reported for this terminal.
    perCheckout({ "checkout:first": [{ id: "ses_only", agent: "coder" }] });
    mocks.listAgentCandidateSessions.mockClear();

    const state = useTerminalAgentRows(computed(() => ["checkout:first"]));
    await settle();

    expect(state.byCheckout["checkout:first"].sessions[0]?.agent?.label).toBe("Coder");
    // One read for the one checkout, asked of that checkout alone: the row cannot be drawn from
    // any other worktree's sessions, so it never asks for them.
    expect(mocks.listAgentCandidateSessions).toHaveBeenCalledTimes(1);
    expect(mocks.listAgentCandidateSessions).toHaveBeenCalledWith("checkout:first");
  });

  it("drops a checkout that no longer has terminals", async () => {
    perCheckout({ "checkout:first": [{ id: "ses_one", agent: "coder" }] });
    const ids = ref(["checkout:first", "checkout:second"]);
    const state = useTerminalAgentRows(computed(() => ids.value));
    await settle();
    expect(state.byCheckout["checkout:first"]).toBeDefined();

    // A closed worktree must stop answering, or its row outlives the terminal it described.
    ids.value = ["checkout:first"];
    await settle();
    expect(Object.keys(state.byCheckout)).toEqual(["checkout:first"]);
  });

  it("reports no agent while the service is not run, without starting one", async () => {
    mocks.listAgentCandidateSessions.mockRejectedValue(new Error("OpenCode is not running."));

    const state = useTerminalAgentRows(computed(() => ["checkout:first"]));
    await settle();

    expect(state.byCheckout["checkout:first"]).toEqual({ sessions: [] });
    expect(state.row("checkout:absent")).toEqual({ sessions: [] });
  });

  it("offers a session with no agent as a session, so a title can still match it", async () => {
    // A fresh session has no agent behind it yet, which is not the same as not existing: its title
    // is what a terminal with it open writes into its own title, and the row can still say which
    // session it is looking at. It just has no mode to name.
    perCheckout({ "checkout:first": [{ id: "ses_fresh", agent: null }] });

    const state = useTerminalAgentRows(computed(() => ["checkout:first"]));
    await settle();

    // The clock rides along because it is the only duration a terminal row has: the elapsed time in a
    // row's trailing slot is this session's own last update, read from the service and nowhere else.
    expect(state.byCheckout["checkout:first"].sessions).toEqual([
      { title: "review", agent: null, running: false, updatedAt: 1 },
    ]);
  });

  it("re-reads fast while a row is running and slowly once every row is idle", async () => {
    vi.useFakeTimers();
    perCheckout({ "checkout:first": [{ id: "ses_one", agent: "coder", running: true }] });

    const state = useTerminalAgentRows(computed(() => ["checkout:first"]));
    await vi.advanceTimersByTimeAsync(0);
    expect(state.byCheckout["checkout:first"].sessions[0]?.running).toBe(true);
    const whileRunning = mocks.listAgentCandidateSessions.mock.calls.length;

    await vi.advanceTimersByTimeAsync(2000);
    expect(mocks.listAgentCandidateSessions.mock.calls.length).toBeGreaterThan(whileRunning);

    perCheckout({ "checkout:first": [{ id: "ses_one", agent: "coder" }] });
    await vi.advanceTimersByTimeAsync(2000);
    expect(state.byCheckout["checkout:first"].sessions[0]?.running).toBe(false);

    // Idle is not the end of the questions. A turn started in the person's own TUI announces nothing
    // to this hook, so a row that stopped asking would say "idle" while the agent works, and nothing
    // would correct it until some unrelated event reloaded the list.
    const onceIdle = mocks.listAgentCandidateSessions.mock.calls.length;
    await vi.advanceTimersByTimeAsync(5000);
    expect(mocks.listAgentCandidateSessions.mock.calls.length).toBeGreaterThan(onceIdle);
    const slow = mocks.listAgentCandidateSessions.mock.calls.length - onceIdle;

    // And it asks at the slow cadence, not the fast one: a second 5s window costs about one request.
    await vi.advanceTimersByTimeAsync(5000);
    expect(mocks.listAgentCandidateSessions.mock.calls.length - onceIdle).toBeLessThanOrEqual(slow + 1);

    // A turn that began while the panel was idle is seen, which is the whole reason for the above.
    perCheckout({ "checkout:first": [{ id: "ses_one", agent: "coder", running: true }] });
    await vi.advanceTimersByTimeAsync(5000);
    expect(state.byCheckout["checkout:first"].sessions[0]?.running).toBe(true);
    vi.useRealTimers();
  });
});
