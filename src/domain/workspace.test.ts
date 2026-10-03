import { describe, expect, it } from "vitest";
import {
  createCheckout,
  createPlainRepo,
  createWorkspaceState,
  getActiveCheckout,
  openRepo,
  selectCheckout,
  selectSession,
  sessionTitle,
  workdirIconKind,
  type Repo,
  type Session,
} from "./workspace";

const openedFolder = (path: string, name = "repo") => ({ path, name });

/** A repo with a root and one worktree, which is the shape the icon rule is asked about. */
function gitRepo(): Repo {
  const repoId = "repo:/work/app";
  return {
    id: repoId,
    kind: "git",
    name: "app",
    root: "/work/app",
    checkouts: [
      createCheckout({ repoId, path: "/work/app", canonicalPath: "/work/app", isPrimary: true, branch: "main" }),
      createCheckout({
        repoId,
        path: "/work/app-feature",
        canonicalPath: "/work/app-feature",
        isPrimary: false,
        branch: "feature",
      }),
    ],
    createdAt: "2026-01-01T00:00:00Z",
    lastOpenedAt: "2026-01-01T00:00:00Z",
  };
}

describe("workspace domain", () => {
  it("models a plain directory as one primary checkout with path-based identity", () => {
    const repo = createPlainRepo(openedFolder("/work/plain", "plain"), "2026-01-01T00:00:00Z");

    expect(repo).toMatchObject({
      id: "repo:/work/plain",
      kind: "plain",
      root: "/work/plain",
      createdAt: "2026-01-01T00:00:00Z",
    });
    expect(repo.checkouts).toHaveLength(1);
    expect(repo.checkouts[0]).toMatchObject({
      id: "checkout:/work/plain",
      repoId: repo.id,
      canonicalPath: "/work/plain",
      isPrimary: true,
      changedFiles: 0,
      sessions: [],
    });
    expect("sessions" in repo).toBe(false);
  });

  it("gives the primary checkout and worktrees the same checkout capabilities", () => {
    const repoId = "repo:/work/app";
    const primary = createCheckout({
      repoId,
      path: "/work/app",
      canonicalPath: "/work/app",
      isPrimary: true,
      branch: "main",
    });
    const worktree = createCheckout({
      repoId,
      path: "/work/app-feature",
      canonicalPath: "/work/app-feature",
      isPrimary: false,
      branch: "feature",
    });
    const repo: Repo = {
      id: repoId,
      kind: "git",
      name: "app",
      root: "/work/app",
      checkouts: [primary, worktree],
      createdAt: "2026-01-01T00:00:00Z",
      lastOpenedAt: "2026-01-01T00:00:00Z",
    };

    expect(primary).toHaveProperty("sessions", []);
    expect(worktree).toHaveProperty("sessions", []);
    expect(primary).toHaveProperty("changedFiles", 0);
    expect(worktree).toHaveProperty("changedFiles", 0);
    expect(repo.checkouts.map(({ isPrimary }) => isPrimary)).toEqual([true, false]);
  });

  it("focuses an existing canonical checkout instead of duplicating a symlink alias", () => {
    const state = openRepo(createWorkspaceState(), createPlainRepo(openedFolder("/work/app", "app"), "first-open"));
    const alias: Repo = {
      ...createPlainRepo(openedFolder("/work/app", "alias"), "reopened"),
      checkouts: [
        createCheckout({
          repoId: "repo:/work/app",
          path: "/work/app-link",
          canonicalPath: "/work/app",
          isPrimary: true,
        }),
      ],
    };

    const reopened = openRepo(state, alias);

    expect(reopened.repos).toHaveLength(1);
    expect(reopened.activeCheckoutId).toBe("checkout:/work/app");
    expect(reopened.repos[0].lastOpenedAt).toBe("reopened");
  });

  it("focuses the specifically opened worktree instead of falling back to primary", () => {
    const repoId = "repo:/work/app";
    const primary = createCheckout({
      repoId,
      path: "/work/app",
      canonicalPath: "/work/app",
      isPrimary: true,
    });
    const worktree = createCheckout({
      repoId,
      path: "/work/app-feature",
      canonicalPath: "/work/app-feature",
      isPrimary: false,
      branch: "feature",
    });
    const repo: Repo = {
      id: repoId,
      kind: "git",
      name: "app",
      root: "/work/app",
      checkouts: [primary, worktree],
      createdAt: "first-open",
      lastOpenedAt: "first-open",
    };

    const opened = openRepo(createWorkspaceState(), repo, worktree.id);

    expect(opened.activeCheckoutId).toBe(worktree.id);
    expect(opened.repos[0].checkouts.map((checkout) => checkout.id)).toEqual([primary.id, worktree.id]);
  });

  it("switches checkouts and sessions independently while keeping sessions owned", () => {
    const repoId = "repo:/work/app";
    const shell = {
      id: "session-shell",
      type: "shell" as const,
      checkoutId: "checkout:/work/app",
      name: "zsh",
      createdAt: "now",
      status: "inactive" as const,
    };
    const server = {
      id: "session-server",
      type: "server" as const,
      checkoutId: "checkout:/work/app-feature",
      name: "vite",
      createdAt: "now",
      status: "inactive" as const,
    };
    const primary = createCheckout({
      repoId,
      path: "/work/app",
      canonicalPath: "/work/app",
      isPrimary: true,
      sessions: [shell],
    });
    const worktree = createCheckout({
      repoId,
      path: "/work/app-feature",
      canonicalPath: "/work/app-feature",
      isPrimary: false,
      sessions: [server],
    });
    const repo: Repo = {
      id: repoId,
      kind: "git",
      name: "app",
      root: "/work/app",
      checkouts: [primary, worktree],
      createdAt: "now",
      lastOpenedAt: "now",
    };

    let state = openRepo(createWorkspaceState(), repo);
    state = selectSession(state, shell.id);
    expect(state).toMatchObject({ activeCheckoutId: primary.id, activeSessionId: shell.id });

    state = selectCheckout(state, worktree.id);
    expect(state).toMatchObject({ activeCheckoutId: worktree.id, activeSessionId: null });
    expect(getActiveCheckout(state)).toBe(worktree);

    state = selectSession(state, server.id);
    expect(state).toMatchObject({ activeCheckoutId: worktree.id, activeSessionId: server.id });
    expect(primary.sessions).toEqual([shell]);
    expect(worktree.sessions).toEqual([server]);
  });

  it("names a session after what is in front of it, and after the name it was opened with", () => {
    const shell: Session = {
      id: "session-shell",
      type: "shell",
      checkoutId: "checkout:/work/app",
      name: "zsh",
      createdAt: "now",
      status: "active",
    };

    // The title a program sets is the name it wants to be known by; the program in front is the
    // next thing down, and the stored name is what is left when nothing is in front at all —
    // which is also what a terminal with no live status has to be named by.
    expect(sessionTitle(shell, { state: "running", terminalTitle: "OpenCode: review task" })).toBe(
      "OpenCode: review task",
    );
    expect(sessionTitle(shell, { state: "running", foregroundProcess: true, foregroundApp: "nvim" })).toBe("nvim");
    expect(sessionTitle(shell, { state: "running", foregroundProcess: false })).toBe("zsh");
    expect(sessionTitle(shell)).toBe("zsh");
  });

  it("gives a checkout the icon its row in the sidebar wears", () => {
    const git = gitRepo();
    const root = git.checkouts[0]!;
    const worktree = git.checkouts[1]!;
    const plain = createPlainRepo(openedFolder("/work/notes", "notes"), "2026-01-01T00:00:00Z");

    // A plain directory is a folder, a repo root is a Git one, and anything else in a repo is a
    // branch checked out in a directory of its own.
    expect(workdirIconKind(plain, plain.checkouts[0]!)).toBe("folder");
    expect(workdirIconKind(git, root)).toBe("git");
    expect(workdirIconKind(git, worktree)).toBe("worktree");
    // The titlebar's first crumb falls back to the folder name when there is no repo to ask.
    expect(workdirIconKind(null, root)).toBe("folder");
  });

  it("decides the missing and the home one before anything the repo says", () => {
    const git = gitRepo();
    const worktree = git.checkouts[1]!;

    // A directory that is gone is still the row it was, and the home one is still the place, so
    // neither of them can be answered further down the order.
    expect(workdirIconKind(git, { ...worktree, isMissing: true }, worktree.id)).toBe("missing");
    expect(workdirIconKind(git, { ...worktree, isMissing: true })).toBe("missing");
    expect(workdirIconKind(git, worktree, worktree.id)).toBe("home");
    expect(workdirIconKind(git, git.checkouts[0]!, git.checkouts[0]!.id)).toBe("home");
  });
});
