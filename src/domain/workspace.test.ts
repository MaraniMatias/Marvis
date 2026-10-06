import { describe, expect, it } from "vitest";
import {
  agentSessionTitle,
  createCheckout,
  createPlainRepo,
  createWorkspaceState,
  getActiveCheckout,
  openRepo,
  selectCheckout,
  selectSession,
  sessionRowTitle,
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
    const worktreeShell = {
      id: "session-worktree-shell",
      type: "shell" as const,
      checkoutId: "checkout:/work/app-feature",
      name: "bash",
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
      sessions: [worktreeShell],
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

    state = selectSession(state, worktreeShell.id);
    expect(state).toMatchObject({ activeCheckoutId: worktree.id, activeSessionId: worktreeShell.id });
    expect(primary.sessions).toEqual([shell]);
    expect(worktree.sessions).toEqual([worktreeShell]);
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

  it("leaves the crumb's own naming alone when the sidebar row changes what it says about a workdir", () => {
    // The two rules are siblings, not one rule: same question, same terminal, and they must not
    // disagree about what it is called — but they are not the same question. The crumb is the
    // window's own title, so it takes whatever the PTY last wrote. The row is a line in a list, so
    // it takes the session and nothing else: the worktree above it already says where the terminal
    // is, and repeating it made two sibling shells read identically.
    const shell: Session = {
      id: "session-shell",
      type: "shell",
      checkoutId: "checkout:/work/app/.worktrees/bug/13133933180-disable-adguard",
      name: "zsh",
      createdAt: "now",
      status: "active",
    };
    const prompt = { state: "running" as const, foregroundProcess: false };

    // The crumb: unchanged, whatever the PTY wrote last, agent prefix and all.
    expect(sessionTitle(shell, { state: "running", terminalTitle: "OC | Copy ids" })).toBe("OC | Copy ids");
    expect(sessionTitle(shell, { state: "running", terminalTitle: "marani@msi:~/Trabajo" })).toBe(
      "marani@msi:~/Trabajo",
    );
    expect(sessionTitle(shell, { state: "running", foregroundProcess: true, foregroundApp: "nvim" })).toBe("nvim");
    expect(sessionTitle(shell, prompt)).toBe("zsh");

    // The row: the session, and never a place. Both read `zsh` here while the crumb above reads the
    // prompt — that is the intended difference, not a drift between them.
    expect(sessionRowTitle(shell, prompt)).toBe("zsh");
    expect(sessionRowTitle(shell, { state: "running", terminalTitle: "OC | Copy ids" })).toBe("zsh");
  });

  it("names a sidebar row by what runs in it, never by the prompt a shell left in the title", () => {
    const shell: Session = {
      id: "session-shell",
      type: "shell",
      checkoutId: "checkout:/work/app",
      name: "zsh",
      createdAt: "now",
      status: "active",
    };
    const prompt = { state: "running" as const, foregroundProcess: false, terminalTitle: "marani@msi:~/Trabajo" };

    // The row names the session and nothing else: the program in front of the shell when there is one,
    // the session's own name when there is not. Never the directory, because the branch row above
    // already carries it and repeating it made two sibling shells read the same text.
    expect(sessionRowTitle(shell, { state: "running", foregroundProcess: true, foregroundApp: "pnpm" })).toBe("pnpm");
    // A program that writes its own window title names no session, and the row does not go mining
    // one out of it: the program in front of the shell is what a terminal is named by, read from the
    // OS rather than guessed out of a string the program chose to write. So an editor reads `zsh` —
    // the shell it runs in — until the OS says what is in front.
    expect(
      sessionRowTitle(shell, { state: "running", foregroundProcess: true, terminalTitle: "nvim — src/main.rs" }),
    ).toBe("zsh");
    // A shell's own prompt is never a row's name either, and neither is the directory it sits in.
    expect(sessionRowTitle(shell, prompt)).toBe("zsh");
    expect(sessionRowTitle(shell, undefined)).toBe("zsh");
    // A rename is the stored name, and it is therefore the row's name whenever nothing else is.
    expect(sessionRowTitle({ ...shell, name: "deploy" }, prompt)).toBe("deploy");

    // The session a terminal is showing is named by that session's own title, which the terminal's
    // TUI wrote and which belongs to this terminal rather than to the directory it sits in. A cut
    // title is still the session's name, and a home screen names no session, so that row falls
    // back to the program. Both answers need the runtime to confirm OpenCode is in front of the
    // shell, or the title is a leftover rather than a fact.
    const agent = { ...shell, id: "session-agent" };
    const opencode = { state: "running" as const, foregroundProcess: true, foregroundApp: "opencode" };
    expect(sessionRowTitle(agent, { ...opencode, terminalTitle: "OC | Copy ids into lists" })).toBe(
      "Copy ids into lists",
    );
    expect(sessionRowTitle(agent, { ...opencode, terminalTitle: "OC | Plan de implementación para la s…" })).toBe(
      "Plan de implementación para la s…",
    );
    expect(sessionRowTitle(agent, { ...opencode, terminalTitle: "OC | OpenCode" })).toBe("opencode");
  });

  it("never reads a leftover agent title once another program is in front", () => {
    // A PTY title outlives the process that wrote it. OpenCode sets `OC | …`, exits, and the shell
    // takes the terminal back without writing anything, so the status still carries the old string.
    // Reading it would name a terminal after an agent no longer running in it and draw that agent's
    // state on it, so the current foreground answer has to say OpenCode before the title is read.
    const stale = "OC | Copy ids into lists";
    expect(agentSessionTitle({ state: "running", foregroundProcess: false, terminalTitle: stale })).toBeNull();
    expect(
      agentSessionTitle({ state: "running", foregroundProcess: true, foregroundApp: "sleep", terminalTitle: stale }),
    ).toBeNull();
    expect(
      agentSessionTitle({ state: "running", foregroundProcess: true, foregroundApp: "nvim", terminalTitle: stale }),
    ).toBeNull();
    expect(agentSessionTitle({ state: "running", terminalTitle: stale })).toBeNull();
    // The same title with the program still in front is still read: the check is about now, not
    // about whether a string ever looked like one.
    expect(
      agentSessionTitle({ state: "running", foregroundProcess: true, foregroundApp: "opencode", terminalTitle: stale }),
    ).toBe("Copy ids into lists");

    // And the row agrees: a terminal running something else is named after that program, and one with
    // nothing in front falls back to the name it was opened with — which is where a rename lives, and
    // which is the last answer rather than the first.
    const renamed: Session = {
      id: "session-renamed",
      type: "shell",
      checkoutId: "checkout:/work/app",
      name: "deploy",
      createdAt: "now",
      status: "active",
    };
    expect(
      sessionRowTitle(renamed, {
        state: "running",
        foregroundProcess: true,
        foregroundApp: "sleep",
        terminalTitle: stale,
      }),
    ).toBe("sleep");
    expect(sessionRowTitle(renamed, { state: "running", foregroundProcess: false, terminalTitle: stale })).toBe(
      "deploy",
    );
  });

  it("reads a session title out of a terminal title only behind OpenCode's own prefix", () => {
    // What a terminal's title can be called a session: what that terminal's own program wrote, and
    // only while that program is the one in front of the shell. The prefix is the other half of the
    // test — a shell's prompt, an editor's window title and a title with no session behind it are
    // all terminal titles, and none of them is a session.
    const inOpenCode = { state: "running" as const, foregroundProcess: true, foregroundApp: "opencode" };
    const of = (terminalTitle: string | null) => agentSessionTitle({ ...inOpenCode, terminalTitle });
    expect(of("OC | Copy ids into lists")).toBe("Copy ids into lists");
    expect(of("OC | OpenCode")).toBeNull();
    expect(of("marani@msi:~/Trabajo")).toBeNull();
    expect(of("nvim — src/main.rs")).toBeNull();
    expect(of(null)).toBeNull();
    expect(agentSessionTitle(undefined)).toBeNull();
    // The separator belongs to the prefix and is not part of the name, and nothing is stripped
    // from the middle of one.
    expect(of("OC |   spaced   ")).toBe("spaced");
    expect(of("OC | OC | nested")).toBe("OC | nested");
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
