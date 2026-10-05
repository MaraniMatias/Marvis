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
import { createAgentSession, listAgentAgents, listAgentSessions, stopAgent } from "../lib/ipc";

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
   * Increments once per turn that was seen to finish. This is the only completion signal
   * v2.0.18 offers, so consumers that need to re-check a finished turn watch this.
   */
  turnsCompleted: number;
  reload(): Promise<boolean>;
  createSession(title: string): Promise<AgentSession | null>;
  selectTarget(sessionId: string | null): void;
  stop(): Promise<void>;
}

const MAX_EVENTS = 200;
/**
 * How often a working session is re-read while the agent is busy.
 *
 * v2.0.18 has no turn-completed event: a successful turn ends in silence, and idleness is
 * only visible as `time.idle` on the session. So a turn ending has to be observed, and this
 * is the cost. Polling stops the moment nothing is busy, so an idle app is not polled.
 */
const BUSY_POLL_MS = 2000;
/**
 * How often the service is asked about again while it is not there.
 *
 * Nothing is started on these ticks: this app is a client of a service the person runs, so the
 * only way to notice one is to ask. See `DISCONNECTED_POLL_MS` at its use.
 */
const DISCONNECTED_POLL_MS = 5000;

function errorText(cause: unknown): string {
  return isIpcError(cause) ? cause.message : cause instanceof Error ? cause.message : String(cause);
}

/**
 * Applies one event to the session list.
 *
 * The server's own truth is authoritative: a turn that started or failed is applied as-is.
 * Idleness is not inferred, because v2.0.18 has no idle event; the list is re-read instead.
 */
export function applyAgentEvent(sessions: AgentSession[], event: AgentEvent): AgentSession[] {
  if (!event.sessionId) return sessions;
  return sessions.map((session) => {
    if (session.id !== event.sessionId) return session;
    switch (event.kind) {
      case "turnStarted":
        return { ...session, busy: true };
      case "turnFailed":
        return { ...session, busy: false };
      case "permissionAsked":
        // This server version cannot answer a permission, so the turn is stuck until the
        // policy changes. Say so instead of pretending the agent is working.
        return { ...session, busy: false, blockedOnPermission: true };
      default:
        return session;
    }
  });
}

/**
 * A session counts as working only once a turn start has been seen for it.
 *
 * A session that has never run has no idle time either, so `idleAt === null` alone would
 * report every fresh session as working and never settle.
 */
function isWorking(session: AgentSession, startedTurns: ReadonlySet<string>): boolean {
  return startedTurns.has(session.id) && session.idleAt === null;
}

/** Recomputes `busy` from the server's idle time and the turns this client saw start. */
function withBusy(sessions: AgentSession[], startedTurns: ReadonlySet<string>): AgentSession[] {
  return sessions.map((session) => ({ ...session, busy: isWorking(session, startedTurns) }));
}

export function useAgentSessions(
  checkout: ComputedRef<Checkout | null>,
  repo: ComputedRef<Repo | null>,
): ActiveAgentSessions {
  let generation = 0;
  /** Sessions whose turn this client saw start and has not yet seen go idle. */
  const startedTurns = new Set<string>();
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
      // A turn this client saw start, that the server now reports as idle, has finished.
      for (const session of sessions) {
        if (session.idleAt !== null && startedTurns.delete(session.id)) state.turnsCompleted += 1;
      }
      state.sessions = withBusy(sessions, startedTurns);
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
      startedTurns.clear();
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
   * Re-reads the sessions while a turn this client saw start is still running.
   *
   * This is the only completion signal v2.0.18 offers, so it is polled rather than
   * announced, and only while there is something to wait for.
   */
  async function pollBusySessions() {
    if (startedTurns.size === 0) return;
    await reload();
  }

  const pollTimer = setInterval(() => void pollBusySessions(), BUSY_POLL_MS);

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
  }, DISCONNECTED_POLL_MS);

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
    if (payload.kind === "turnStarted" && payload.sessionId) {
      startedTurns.add(payload.sessionId);
      state.sessions = state.sessions.map((session) =>
        session.id === payload.sessionId ? { ...session, busy: true } : session,
      );
      // The turn is known to be running, so start watching for it to end.
      void reload();
      return;
    }
    if (isTurnEvent(payload.kind)) {
      // The server's own view is the authority, and it lags the event, so the list is
      // re-read and that transition is what settles a finished turn.
      void reload();
      return;
    }
    state.sessions = applyAgentEvent(state.sessions, payload);
  });

  return Object.assign(state, { reload, createSession, selectTarget, stop });
}
