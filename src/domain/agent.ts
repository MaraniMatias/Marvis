export type AgentEventKind =
  "unknown" | "turnStarted" | "turnFinished" | "turnFailed" | "toolCalled" | "permissionAsked";

/** One OpenCode session in a checkout. `id` is only valid inside that checkout's server. */
export interface AgentSession {
  id: string;
  checkoutId: string;
  title: string;
  /**
   * True while a turn is running.
   *
   * Derived, not reported: the server only says when a session last went idle, and "no idle
   * time yet" covers both a session that never ran and one working right now. Only a turn
   * this client saw start can be called working.
   */
  busy: boolean;
  /** When the server last saw this session go idle, if ever. */
  idleAt: number | null;
  /** Set when a permission arrived that this server version cannot answer. */
  blockedOnPermission: boolean;
  /** The agent running it, which is also the mode: OpenCode spells `build`/`plan` as agents. */
  agent: string | null;
  /** The model behind it, as `provider/id#variant`. */
  model: string | null;
  /** The session a subagent was spawned from. */
  parentId: string | null;
  /** How the last turn ended: `succeeded`, `failed` or `interrupted`. */
  outcome: string | null;
  createdAt: number;
  updatedAt: number;
}

/** One agent a checkout's server offers, as OpenCode describes it. */
export interface AgentAgent {
  id: string;
  name: string;
  /** `primary` is a mode a person picks, `subagent` is one a parent spawns, `all` is both. */
  mode: string;
  /** The hex OpenCode paints this agent with, absent when it has no color of its own. */
  color: string | null;
  /** Internal agents: real, and not something to offer as a choice. */
  hidden: boolean;
}

export interface AgentEvent {
  checkoutId: string;
  sessionId: string | null;
  kind: AgentEventKind;
  /** The server's own event type, kept for diagnostics. */
  rawType: string;
  data: unknown;
}

const turnEvents: AgentEventKind[] = ["turnStarted", "turnFinished", "turnFailed"];

/** How loudly a session wants for attention, worst last. */
export type AgentAttention = "none" | "busy" | "blocked" | "failed";

/** An agent session is busy, blocked, failed, or simply not there. */
export function agentAttention(session: AgentSession | undefined): AgentAttention {
  if (!session) return "none";
  if (session.blockedOnPermission) return "blocked";
  if (session.busy) return "busy";
  // A turn the server ended badly is the last word, so it outranks a quiet session.
  return session.outcome === "failed" ? "failed" : "none";
}

const HEX_COLOR = /^#(?:[0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

/**
 * The agent's own color, when it is one a stylesheet can take.
 *
 * The value is the server's, and it is about to become a `background`, so it is checked
 * against the hex OpenCode actually writes rather than trusted. Anything else is no color,
 * and the row falls back to the accent.
 */
export function agentColor(agents: AgentAgent[], agentId: string | null | undefined): string | null {
  if (!agentId) return null;
  const color = agents.find((agent) => agent.id === agentId)?.color;
  return color && HEX_COLOR.test(color) ? color : null;
}

/** The display name of an agent, falling back to its id when the server names it nothing. */
export function agentLabel(agents: AgentAgent[], agentId: string | null | undefined): string | null {
  if (!agentId) return null;
  const agent = agents.find((candidate) => candidate.id === agentId);
  return agent?.name || agentId;
}

/** Sorts so the most recently touched session is first: the default send target. */
export function sortAgentSessions(sessions: AgentSession[]): AgentSession[] {
  return [...sessions].sort((first, second) => second.updatedAt - first.updatedAt || first.id.localeCompare(second.id));
}

/** The session a review round should go to: the most recent live one, else the newest. */
export function defaultAgentSession(sessions: AgentSession[]): AgentSession | null {
  const sorted = sortAgentSessions(sessions);
  return sorted[0] ?? null;
}

/**
 * The one session a row speaks for.
 *
 * A session that wants attention outranks a quiet one, because that is the row's news; among
 * equals the most recently touched wins, and with nothing to report the round's own target is
 * named so the row agrees with where a review would go.
 *
 * A session that never ran has no agent, and a row cannot name nothing, so those are only
 * considered when there is no session to name: a fresh one is the newest by `updatedAt` and
 * would otherwise blank a row that has a perfectly good agent to show.
 */
export function headlineSession(sessions: AgentSession[], targetId: string | null): AgentSession | null {
  const nameable = sessions.filter((session) => session.agent);
  const loud = nameable.filter((session) => agentAttention(session) !== "none");
  if (loud.length) return sortAgentSessions(loud)[0] ?? null;
  return nameable.find((session) => session.id === targetId) ?? defaultAgentSession(nameable);
}

export function isTurnEvent(kind: AgentEventKind): boolean {
  return turnEvents.includes(kind);
}
