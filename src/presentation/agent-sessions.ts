import { listen } from "@tauri-apps/api/event";
import { onScopeDispose, reactive, watch } from "vue";
import type { ComputedRef } from "vue";
import type { AgentAttention, AgentAgent, AgentEvent, AgentSession } from "../domain/agent";
import {
  agentAttention,
  agentColor,
  agentLabel,
  defaultAgentSession,
  headlineSession,
  isTurnEvent,
} from "../domain/agent";
import { isIpcError } from "../domain/ipc";
import type { Checkout, Repo } from "../domain/workspace";
import {
  createAgentSession,
  listAgentAgents,
  listAgentCandidateSessions,
  listAgentSessions,
  stopAgent,
} from "../lib/ipc";

/** Name of the Tauri event the bridge emits normalized agent events on. */
export const AGENT_EVENT = "marvis://agent-event";

/** What a row says about the agent: which one, in OpenCode's color, and how loudly. */
export interface AgentHeadline {
  label: string;
  color: string | null;
  attention: AgentAttention;
}

export interface ActiveAgentSessions {
  checkoutId: string | null;
  sessions: AgentSession[];
  /** Every agent the checkout's server offers, for the colors and names above. */
  agents: AgentAgent[];
  /** The one session a row would speak for, already resolved to a name and a color. */
  headline: AgentHeadline | null;
  /** The session a review round goes to unless the user picks another. */
  targetId: string | null;
  state: "loading" | "ready" | "error";
  error: string;
  /** Live events for the active checkout, newest last. */
  events: AgentEvent[];
  /**
   * Increments once per turn that was seen to finish. The service announces no completion, so
   * consumers that need to re-check a finished turn watch this.
   */
  turnsCompleted: number;
  reload(): Promise<boolean>;
  createSession(title: string): Promise<AgentSession | null>;
  selectTarget(sessionId: string | null): void;
  stop(): Promise<void>;
}

const MAX_EVENTS = 200;
/**
 * How often a working session is re-read while the service reports a turn running.
 *
 * The service announces no turn-completed event: a turn ends in silence and only
 * `/api/session/active` stops reporting it. So an ending has to be observed, and this is the cost.
 */
const BUSY_POLL_MS = 2000;
/**
 * How often the service is asked about again on the slow question.
 *
 * Both hooks that use it are asking something a person decides on their own schedule — whether they
 * have started OpenCode, whether a turn has begun — and neither is answered by an event this app
 * receives. Named for the cadence rather than for one of its two callers.
 */
const SLOW_POLL_MS = 5000;

function errorText(cause: unknown): string {
  return isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
}

/**
 * Counts the turns that ended since the last read, and leaves `wasRunning` holding what is
 * running now.
 *
 * `wasRunning` is updated in place because it is the only record of what the previous read said:
 * a session that was running and is not has finished, and one that was not and is has started.
 */
function settledTurns(sessions: AgentSession[], wasRunning: Set<string>): number {
  let settled = 0;
  for (const session of sessions) {
    if (session.running) {
      wasRunning.add(session.id);
    } else if (wasRunning.delete(session.id)) {
      settled += 1;
    }
  }
  // A session the service no longer lists cannot be running, so it cannot still be owed a turn.
  const listed = new Set(sessions.map((session) => session.id));
  for (const sessionId of [...wasRunning]) {
    if (!listed.has(sessionId)) wasRunning.delete(sessionId);
  }
  return settled;
}

/**
 * Records what the service reports without counting anything, for the first read of a checkout.
 *
 * The first read has nothing to compare against, so it only seeds the record. Skipping this would
 * leave a turn that was already running when the checkout was opened looking like it had never
 * run, and its ending would go unnoticed.
 */
function recordRunning(sessions: AgentSession[], wasRunning: Set<string>): void {
  wasRunning.clear();
  for (const session of sessions) {
    if (session.running) wasRunning.add(session.id);
  }
}

/**
 * Applies one event to the session list.
 *
 * The server's own truth is authoritative and is re-read on every turn event, because
 * `/api/session/active` is what decides `running`. An event only moves the row between reads:
 * a turn that started shows as working at once instead of at the next poll, and one that ended
 * is left to the re-read rather than being declared finished here.
 */
export function applyAgentEvent(sessions: AgentSession[], event: AgentEvent): AgentSession[] {
  if (!event.sessionId) return sessions;
  return sessions.map((session) => {
    if (session.id !== event.sessionId) return session;
    switch (event.kind) {
      case "turnStarted":
        return { ...session, running: true };
      case "permissionAsked":
        // This server version cannot answer a permission, so the turn is stuck until the
        // policy changes. Say so instead of pretending the agent is working.
        return { ...session, running: false, blockedOnPermission: true };
      default:
        return session;
    }
  });
}

export function useAgentSessions(
  checkout: ComputedRef<Checkout | null>,
  repo: ComputedRef<Repo | null>,
): ActiveAgentSessions {
  let generation = 0;
  /**
   * Sessions the service reported as running at the previous read.
   *
   * A turn ending is the transition out of that set, which is how a finished turn is noticed:
   * the service announces no completion event, and this also counts a turn the person ran in
   * their own TUI, which no event on this stream would ever have carried.
   */
  const wasRunning = new Set<string>();
  const state = reactive<Omit<ActiveAgentSessions, "reload" | "createSession" | "selectTarget" | "stop">>({
    checkoutId: null,
    sessions: [],
    agents: [],
    headline: null,
    targetId: null,
    state: "ready",
    error: "",
    events: [],
    turnsCompleted: 0,
  });

  /**
   * Resolves the row's line from the session list and the catalog together.
   *
   * It is a watch rather than a computed because the whole of `state` is reactive already, and
   * the row must not be able to disagree with the list it was drawn from.
   */
  function syncHeadline() {
    const session = headlineSession(state.sessions, state.targetId);
    const label = agentLabel(state.agents, session?.agent);
    state.headline =
      session && label
        ? { label, color: agentColor(state.agents, session.agent), attention: agentAttention(session) }
        : null;
  }

  watch(() => [state.sessions, state.agents, state.targetId], syncHeadline);

  function fail(cause: unknown): false {
    state.error = errorText(cause);
    state.state = "error";
    return false;
  }

  /**
   * Re-reads the catalog when a session names an agent it does not have.
   *
   * The server fills that list in stages: right after boot it holds its own built-in agents and
   * the ones from the project, and the ones from the user's agent directory arrive later. Read
   * once per checkout that would leave a row without its color for the rest of the session, so
   * the gap is what triggers the read. In the steady state this asks for nothing.
   */
  async function refreshAgentsBehind(sessions: AgentSession[]) {
    const checkoutId = state.checkoutId;
    if (!checkoutId) return;
    const known = new Set(state.agents.map((agent) => agent.id));
    if (!sessions.some((session) => session.agent && !known.has(session.agent))) return;
    try {
      const agents = await listAgentAgents(checkoutId);
      // A newer checkout may have taken over while this was in flight.
      if (state.checkoutId === checkoutId) state.agents = agents;
    } catch {
      // A catalog that cannot be read leaves the row with the accent, which it already had.
    }
  }

  async function reload() {
    const checkoutId = state.checkoutId;
    if (!checkoutId) return false;
    const request = ++generation;
    try {
      const sessions = await listAgentSessions(checkoutId);
      if (state.checkoutId !== checkoutId || request !== generation) return false;
      state.turnsCompleted += settledTurns(sessions, wasRunning);
      state.sessions = sessions;
      // Keep an explicit choice only while it still exists; otherwise follow the newest.
      if (!state.targetId || !state.sessions.some((session) => session.id === state.targetId)) {
        state.targetId = defaultAgentSession(state.sessions)?.id ?? null;
      }
      state.state = "ready";
      state.error = "";
      await refreshAgentsBehind(sessions);
      return true;
    } catch (cause) {
      return state.checkoutId === checkoutId ? fail(cause) : false;
    }
  }

  async function createSession(title: string) {
    const checkoutId = state.checkoutId;
    if (!checkoutId) return null;
    try {
      const created = await createAgentSession(checkoutId, title);
      if (state.checkoutId !== checkoutId) return null;
      state.sessions = [created, ...state.sessions];
      state.targetId = created.id;
      state.error = "";
      return created;
    } catch (cause) {
      fail(cause);
      return null;
    }
  }

  function selectTarget(sessionId: string | null) {
    state.targetId = sessionId;
  }

  async function stop() {
    const checkoutId = state.checkoutId;
    if (!checkoutId) return;
    try {
      await stopAgent(checkoutId);
      state.sessions = [];
      state.targetId = null;
      state.error = "";
    } catch (cause) {
      fail(cause);
    }
  }

  watch(
    [() => checkout.value?.id, () => checkout.value?.isMissing, () => repo.value?.kind],
    async ([checkoutId, isMissing, repoKind], _previous, onCleanup) => {
      const requestGeneration = ++generation;
      let current = true;
      onCleanup(() => {
        current = false;
      });
      state.checkoutId = checkoutId ?? null;
      state.sessions = [];
      // The catalog is the same for every session in a checkout, so it is read once per
      // checkout and left alone: a name or a color that changed on disk is not worth a poll.
      state.agents = [];
      state.targetId = null;
      state.events = [];
      wasRunning.clear();
      state.error = "";
      state.state = "ready";
      if (!checkoutId || isMissing || repoKind !== "git") return;

      state.state = "loading";
      try {
        // Best effort: an agent with no catalog is a row without a color, not a row in error.
        const [sessions, agents] = await Promise.all([
          listAgentSessions(checkoutId),
          listAgentAgents(checkoutId).catch(() => [] as AgentAgent[]),
        ]);
        if (!current || requestGeneration !== generation) return;
        state.agents = agents;
        state.sessions = sessions;
        // Seeded rather than counted: there is no previous read for this checkout to differ from.
        recordRunning(sessions, wasRunning);
        state.targetId = defaultAgentSession(sessions)?.id ?? null;
        state.state = "ready";
        await refreshAgentsBehind(sessions);
      } catch (cause) {
        if (!current || requestGeneration !== generation) return;
        state.sessions = [];
        state.state = "error";
        state.error = errorText(cause);
      }
    },
    { immediate: true },
  );

  /**
   * Re-reads the sessions while the service reports one of them running.
   *
   * The service announces no completion event, so a turn ending is a transition seen between two
   * reads. This polls only while something is running, so an idle app is not polled, and it
   * covers a turn started in the person's own TUI, which is the common case here.
   */
  async function pollRunningSessions() {
    if (!state.sessions.some((session) => session.running)) return;
    await reload();
  }

  const pollTimer = setInterval(() => void pollRunningSessions(), BUSY_POLL_MS);

  /**
   * Asks again while there is no service to talk to.
   *
   * The service belongs to whoever started it, so this app does not start one: it waits here until
   * that person runs OpenCode, and connects on the first read that answers. The interval is slower
   * than the busy poll because a person starts OpenCode on their own schedule, not between two
   * turns, and every one of these is a request that answers nothing.
   */
  const disconnectedTimer = setInterval(() => {
    if (state.state !== "error") return;
    void reload();
  }, SLOW_POLL_MS);

  onScopeDispose(() => {
    clearInterval(pollTimer);
    clearInterval(disconnectedTimer);
  });

  // One subscription for the app's lifetime: events are filtered by checkout, so a single
  // listener is cheaper than one per checkout and cannot be left dangling.
  void listen<AgentEvent>(AGENT_EVENT, ({ payload }) => {
    const checkoutId = state.checkoutId;
    if (!checkoutId || payload.checkoutId !== checkoutId) return;
    state.events = [...state.events, payload].slice(-MAX_EVENTS);
    if (isTurnEvent(payload.kind)) {
      // `/api/session/active` is the authority on whether a turn is running and it lags the
      // event, so the list is re-read and that read is what settles a finished turn.
      void reload();
      return;
    }
    state.sessions = applyAgentEvent(state.sessions, payload);
  });

  return Object.assign(state, { reload, createSession, selectTarget, stop });
}

/** What one session says about itself, for a terminal to be matched against it. */
export interface TerminalAgentSession {
  /** The session's own title, which is what its TUI writes into the terminal title. */
  title: string;
  /** The agent running it, with the color OpenCode paints it with, absent when the session has none. */
  agent: AgentHeadline | null;
  /** Whether the service reports a turn running in this session right now. */
  running: boolean;
  /**
   * When the service last touched this session, in epoch milliseconds.
   *
   * Carried because it is the only real clock the row has: the elapsed time in a terminal's right
   * slot is how long ago this session was last updated, read from the service and from nowhere
   * else. Nothing else is a duration this app observed, so nothing else may be drawn as one.
   */
  updatedAt: number;
}

/** What one checkout's sessions say, which is everything a terminal in it could be matched to. */
export interface TerminalAgentRow {
  /**
   * Every session this checkout's service is running.
   *
   * A list rather than one headline on purpose: OpenCode 2.0.22 has no route that maps a TUI
   * process to a session, so the only thing that can say which session a terminal has open is the
   * title that terminal's own TUI wrote into it. That is a title to match against, and it needs
   * every candidate to match against. Empty when the service cannot be reached, which is a list
   * that says nothing rather than a session that says the wrong thing.
   */
  readonly sessions: TerminalAgentSession[];
}

export interface TerminalAgentRows {
  /** Keyed by checkout id, which is what a terminal row knows about itself. */
  readonly byCheckout: Record<string, TerminalAgentRow>;
  row(checkoutId: string): TerminalAgentRow;
  reload(): Promise<void>;
}

/**
 * Reads the agent state of every checkout that has terminals, keyed by checkout.
 *
 * `useAgentSessions` follows the *active* checkout, while a sidebar row is about one terminal in
 * one worktree: two terminals in different worktrees are different rows and must not share one
 * checkout's answer. Each checkout is read on its own and never merged, so a row can only ever be
 * drawn from its own worktree's sessions.
 *
 * It deliberately does not decide which session a given terminal has open. OpenCode 2.0.22 has no
 * route, header or event mapping a TUI process to a session, so that question has no answer to
 * read; see the note at the top of `services/agent.rs`. What *is* per terminal is the directory it
 * runs in, and that is what is scoped here.
 */
export function useTerminalAgentRows(checkoutIds: ComputedRef<string[]>): TerminalAgentRows {
  let generation = 0;
  const byCheckout = reactive<Record<string, TerminalAgentRow>>({});
  /** Checkouts with a turn running, so the poll knows there is something to wait for. */
  const busy = new Set<string>();

  const row = (checkoutId: string): TerminalAgentRow => byCheckout[checkoutId] ?? { sessions: [] };

  async function read(checkoutId: string): Promise<TerminalAgentRow> {
    // The repository-wide list, not the worktree's own: a terminal's session is not necessarily
    // located in the worktree the terminal is filed under, and a row that cannot name its session
    // draws no state at all. Nothing here is trusted on its own — the row matches a title and refuses
    // an ambiguous one.
    const sessions = await listAgentCandidateSessions(checkoutId);
    const agents = await listAgentAgents(checkoutId).catch(() => [] as AgentAgent[]);
    // Every session is offered, and no headline is picked: the terminal row is about one terminal,
    // and the only thing that can say which session that terminal has open is the title the
    // terminal's own TUI wrote into it. It matches against this list by title
    // (`agentSessionForTitle`), and an unmatched terminal draws no state at all.
    return {
      sessions: sessions.map((session) => {
        const label = agentLabel(agents, session.agent);
        return {
          title: session.title,
          agent: label ? { label, color: agentColor(agents, session.agent), attention: agentAttention(session) } : null,
          running: session.running,
          updatedAt: session.updatedAt,
        };
      }),
    };
  }

  async function reload() {
    const request = ++generation;
    // A checkout that went away must stop answering, or a closed worktree keeps its row.
    const wanted = [...new Set(checkoutIds.value)];
    for (const checkoutId of Object.keys(byCheckout)) {
      if (!wanted.includes(checkoutId)) delete byCheckout[checkoutId];
    }
    const read_ = await Promise.all(
      wanted.map(async (checkoutId) => {
        try {
          return [checkoutId, await read(checkoutId)] as const;
        } catch {
          // A service that is not run leaves the row with no session to match, which is the state
          // to wait in rather than a failure to report.
          return [checkoutId, { sessions: [] as TerminalAgentSession[] }] as const;
        }
      }),
    );
    // A reload that was superseded mid-flight must not publish over the newer one.
    if (request !== generation) return;
    for (const [checkoutId, entry] of read_) byCheckout[checkoutId] = entry;
    busy.clear();
    for (const [checkoutId, entry] of read_) {
      if (entry.sessions.some((session) => session.running)) busy.add(checkoutId);
    }
  }

  watch(checkoutIds, () => void reload(), { immediate: true });

  /**
   * Re-reads while anything is running.
   *
   * The service announces no turn-completed event, so this is how a row notices a turn ending,
   * and it covers a turn the person started in their own TUI. It stops the moment every row is
   * idle, so an idle app is not polled.
   */
  const pollTimer = setInterval(() => {
    if (busy.size > 0) void reload();
  }, BUSY_POLL_MS);

  /**
   * Asks again whenever nothing is running, and slowly.
   *
   * The service announces no turn-started event to this hook, and a turn started in the user's own TUI
   * is the one nobody here can hear about. So a panel with sessions on it and nothing running still has
   * to look: otherwise the row says idle while the agent has been working for minutes, and nothing
   * would correct it until some other event happened to reload the list. It was worse than quiet before
   * — a checkout with any session at all stopped both timers, so a service with fifty finished
   * sessions was never asked about again.
   *
   * The two timers are a fast one and a slow one rather than two questions, because the fast poll is
   * only ever the right question while something is running and the slow one is the only right question
   * once it is not.
   */
  const idleTimer = setInterval(() => {
    if (busy.size > 0) return;
    void reload();
  }, SLOW_POLL_MS);

  onScopeDispose(() => {
    clearInterval(pollTimer);
    clearInterval(idleTimer);
  });

  return { byCheckout, row, reload };
}
