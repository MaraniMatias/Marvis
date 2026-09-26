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
  createdAt: number;
  updatedAt: number;
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

/** An agent session is busy, blocked, or simply not there. */
export function agentAttention(session: AgentSession | undefined): "none" | "busy" | "blocked" {
  if (!session) return "none";
  if (session.blockedOnPermission) return "blocked";
  return session.busy ? "busy" : "none";
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

export function isTurnEvent(kind: AgentEventKind): boolean {
  return turnEvents.includes(kind);
}
