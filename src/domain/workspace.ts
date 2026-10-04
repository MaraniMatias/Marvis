import type { OpenedFolder } from "./folder";

export type RepoKind = "git" | "plain";
export type SessionType = "shell" | "agent";
export type SessionStatus = "active" | "inactive";
export type TerminalProcessState = "running" | "exited";

export interface TerminalSessionStatus {
  state: TerminalProcessState;
  exitCode?: number;
  foregroundProcess?: boolean;
  /** The program in front of the shell, by name: `opencode`, `nvim`. Absent when it is idle. */
  foregroundApp?: string;
  /** Title most recently set by the PTY via OSC 0/2, if any. */
  terminalTitle?: string | null;
}

export interface Repo {
  id: string;
  kind: RepoKind;
  name: string;
  root: string;
  defaultBranch?: string;
  checkouts: Checkout[];
  createdAt: string;
  lastOpenedAt: string;
}

export interface Checkout {
  id: string;
  repoId: string;
  path: string;
  canonicalPath: string;
  isPrimary: boolean;
  branch?: string;
  head?: string;
  aheadOfDefault?: number;
  changedFiles: number;
  isMissing: boolean;
  sessions: Session[];
}

export interface Session {
  id: string;
  type: SessionType;
  checkoutId: string;
  name: string;
  createdAt: string;
  status: SessionStatus;
}

/**
 * A worktree that is registered and alive but off the panel.
 *
 * It is what archiving leaves behind: the row goes, the directory and its branch stay,
 * and the repo root is the one row that can put it back.
 */
export interface ArchivedCheckout {
  id: string;
  repoId: string;
  path: string;
  branch?: string;
}

export interface WorkspaceState {
  repos: Repo[];
  /**
   * The worktrees that could be put back, so the repo root can offer them.
   *
   * Absent when there are none, which is every workspace that has never archived one.
   */
  archivedWorktrees?: ArchivedCheckout[];
  activeCheckoutId: string | null;
  activeSessionId: string | null;
  homeCheckoutId?: string | null;
}

/** A folder Marvis has opened before, as the workdir menu lists it. */
export interface RecentPath {
  canonicalPath: string;
  lastOpenedAt: string;
}

/**
 * How a checkout is named wherever it is listed.
 *
 * Git checkouts are named by their branch, with "Base" for a detached repo root. Plain
 * workdirs use their repo name so the sidebar and titlebar agree.
 */
export function workdirTitle(repo: Repo, checkout: Checkout): string {
  return checkout.branch || (repo.kind === "plain" ? repo.name : checkout.isPrimary ? "Base" : checkout.path);
}

export function repoIdForPath(canonicalPath: string): string {
  return `repo:${canonicalPath}`;
}

export function checkoutIdForPath(canonicalPath: string): string {
  return `checkout:${canonicalPath}`;
}

export function createCheckout(
  input: Omit<Checkout, "id" | "changedFiles" | "isMissing" | "sessions"> &
    Partial<Pick<Checkout, "changedFiles" | "isMissing" | "sessions">>,
): Checkout {
  return {
    ...input,
    id: checkoutIdForPath(input.canonicalPath),
    changedFiles: input.changedFiles ?? 0,
    isMissing: input.isMissing ?? false,
    sessions: input.sessions ?? [],
  };
}

export function createPlainRepo(folder: OpenedFolder, now = new Date().toISOString()): Repo {
  const checkout = createCheckout({
    repoId: repoIdForPath(folder.path),
    path: folder.path,
    canonicalPath: folder.path,
    isPrimary: true,
  });

  return {
    id: repoIdForPath(folder.path),
    kind: "plain",
    name: folder.name,
    root: folder.path,
    checkouts: [checkout],
    createdAt: now,
    lastOpenedAt: now,
  };
}

export function createWorkspaceState(): WorkspaceState {
  return { repos: [], activeCheckoutId: null, activeSessionId: null };
}

export function openRepo(
  state: WorkspaceState,
  repo: Repo,
  focusedCheckoutId: string | null = repo.checkouts[0]?.id ?? null,
): WorkspaceState {
  const existingRepo = state.repos.find(
    (knownRepo) =>
      knownRepo.id === repo.id ||
      knownRepo.checkouts.some((checkout) => repo.checkouts.some((opened) => opened.id === checkout.id)),
  );
  const repoId = existingRepo?.id ?? repo.id;
  const checkouts = repo.checkouts.map((opened) => {
    const existing = existingRepo?.checkouts.find((checkout) => checkout.id === opened.id);
    if (opened.repoId === repoId && (!existing || opened.sessions === existing.sessions)) return opened;
    return { ...opened, repoId, ...(existing ? { sessions: existing.sessions } : {}) };
  });
  const resolvedRepo = {
    ...repo,
    id: repoId,
    checkouts,
  };
  const repos = existingRepo
    ? state.repos.map((knownRepo) =>
        knownRepo.id === existingRepo.id ? { ...resolvedRepo, createdAt: knownRepo.createdAt } : knownRepo,
      )
    : [...state.repos, resolvedRepo];
  return selectCheckout({ ...state, repos }, focusedCheckoutId);
}

export function selectCheckout(state: WorkspaceState, checkoutId: string | null): WorkspaceState {
  if (checkoutId === null) return { ...state, activeCheckoutId: null, activeSessionId: null };
  const checkout = state.repos.flatMap((repo) => repo.checkouts).find((item) => item.id === checkoutId);
  if (!checkout) return state;

  return {
    ...state,
    activeCheckoutId: checkout.id,
    activeSessionId: checkout.id === state.activeCheckoutId ? state.activeSessionId : null,
  };
}

export function selectSession(state: WorkspaceState, sessionId: string): WorkspaceState {
  for (const repo of state.repos) {
    const checkout = repo.checkouts.find((item) => item.sessions.some((session) => session.id === sessionId));
    if (checkout) return { ...state, activeCheckoutId: checkout.id, activeSessionId: sessionId };
  }
  return state;
}

export function getActiveCheckout(state: WorkspaceState): Checkout | null {
  return (
    state.repos.flatMap((repo) => repo.checkouts).find((checkout) => checkout.id === state.activeCheckoutId) ?? null
  );
}
