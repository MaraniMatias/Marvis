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
  type Repo,
  type Session,
} from "./workspace";

const openedFolder = (path: string, name = "repo") => ({ path, name });

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
});
