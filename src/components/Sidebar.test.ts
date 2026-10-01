// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ArchivedCheckout, Checkout, Repo, Session } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  getGitCheckoutDiffStats: vi.fn(),
  getGitDiffStats: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  getGitCheckoutDiffStats: mocks.getGitCheckoutDiffStats,
  getGitDiffStats: mocks.getGitDiffStats,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));

import Sidebar from "./Sidebar.vue";

function repo(overrides: Partial<Repo> = {}): Repo {
  return {
    id: "repo:test",
    kind: "git",
    name: "test",
    root: "/test",
    checkouts: [],
    createdAt: "now",
    lastOpenedAt: "now",
    ...overrides,
  };
}

function checkout(overrides: Partial<Checkout> = {}): Checkout {
  return {
    id: "checkout:primary",
    repoId: "repo:test",
    path: "/test",
    canonicalPath: "/test",
    isPrimary: true,
    branch: "main",
    changedFiles: 0,
    isMissing: false,
    sessions: [],
    ...overrides,
  };
}

function session(id: string, name: string, checkoutId = "checkout:primary"): Session {
  return { id, type: "shell", checkoutId, name, createdAt: "now", status: "inactive" };
}

describe("Sidebar workdir rows", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getGitCheckoutDiffStats.mockResolvedValue({});
    mocks.getGitDiffStats.mockResolvedValue([]);
  });

  it("groups checkouts by repo and ends with Open directory", async () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });

    expect(wrapper.find("h2").exists()).toBe(false);
    expect(wrapper.find('input[aria-label="Search repositories"]').exists()).toBe(false);
    expect(wrapper.find(".group-header").exists()).toBe(false);
    expect(wrapper.text()).toContain("Your opened folders will appear here.");
    const openDirectory = wrapper.get('button[aria-label="Open directory"]');
    expect(openDirectory.attributes("title")).toBe("Open directory");
    expect(openDirectory.text()).toBe("Open directory");
    expect(openDirectory.classes()).toContain("new-item");
    await openDirectory.trigger("click");
    expect(wrapper.emitted("openFolder")).toHaveLength(1);
  });

  it("names the repo root after its branch and keeps worktree actions apart", async () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              {
                id: "checkout:primary",
                repoId: "repo:test",
                path: "/test",
                canonicalPath: "/test",
                isPrimary: true,
                branch: "main",
                changedFiles: 0,
                isMissing: false,
                sessions: [],
              },
              {
                id: "checkout:feature",
                repoId: "repo:test",
                path: "/test-feature",
                canonicalPath: "/test-feature",
                isPrimary: false,
                branch: "feature",
                changedFiles: 0,
                isMissing: false,
                sessions: [],
              },
            ],
          }),
        ],
        activeCheckoutId: "checkout:feature",
        activeSessionId: null,
        isOpening: false,
      },
    });

    expect(wrapper.get(".group-header").text()).toBe("test");
    // The repo root is named by the branch it is on, like a worktree is, and the branch is
    // never repeated beside it.
    expect(wrapper.get(".workdir-item .workdir-name").text()).toBe("main");
    expect(wrapper.find(".workdir-branch").exists()).toBe(false);
    // Hover says the whole name, which the row cannot fit, and where the checkout lives.
    expect(wrapper.get(".workdir-item.active .workdir-select").attributes("title")).toBe("feature — /test-feature");

    expect(wrapper.find('button[aria-label="Add worktree from main"]').exists()).toBe(true);
    expect(wrapper.find('button[aria-label="Remove or archive worktree main"]').exists()).toBe(false);
    expect(wrapper.find('button[aria-label="Remove or archive worktree feature"]').exists()).toBe(true);
    // A workdir with nothing open still offers a terminal: picking a workdir no longer opens
    // one by itself, so this row is the way in.
    expect(wrapper.find('button[aria-label="New terminal for main"]').exists()).toBe(true);
    expect(wrapper.find('button[aria-label="New terminal for feature"]').exists()).toBe(true);
    expect(wrapper.text()).not.toContain("Plain");

    await wrapper.get(".workdir-item.active .workdir-select").trigger("click");
    await wrapper.get('button[aria-label="Add worktree from main"]').trigger("click");
    await wrapper.get('button[aria-label="Remove or archive worktree feature"]').trigger("click");
    await wrapper.get('button[aria-label="New terminal for main"]').trigger("click");

    expect(wrapper.emitted("selectCheckout")).toEqual([["checkout:feature", false]]);
    expect(wrapper.emitted("createWorktree")).toEqual([["checkout:primary"]]);
    expect(wrapper.emitted("removeWorktree")).toEqual([["checkout:feature"]]);
    expect(wrapper.emitted("newTerminal")).toEqual([["checkout:primary"]]);
  });

  it("keeps the plain name for a git root Git has no branch to name it by", () => {
    // A detached HEAD is the only checkout Git reports without a branch, and the row still has
    // to be called something: "Base", the same as a plain folder.
    const wrapper = mount(Sidebar, {
      props: {
        repos: [repo({ checkouts: [{ ...checkout(), branch: undefined }] })],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });

    expect(wrapper.get(".workdir-name").text()).toBe("Base");
    expect(wrapper.find(".workdir-branch").exists()).toBe(false);
    expect(wrapper.find('button[aria-label="New terminal for Base"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("gives a plain folder neither worktree action", () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            id: "repo:notes",
            kind: "plain",
            name: "notes",
            root: "/notes",
            checkouts: [
              {
                id: "checkout:notes",
                repoId: "repo:notes",
                path: "/notes",
                canonicalPath: "/notes",
                isPrimary: true,
                changedFiles: 0,
                isMissing: false,
                sessions: [],
              },
            ],
          }),
        ],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });

    // A plain folder is one checkout of its own, so it can be taken off the panel; what it
    // never has is a worktree action, which belongs to Git alone.
    expect(wrapper.get(".workdir-actions button").attributes("aria-label")).toBe("Remove from list: Base");
    expect(wrapper.find("button[aria-label^='Add worktree']").exists()).toBe(false);
    expect(wrapper.find("button[aria-label^='Remove or archive']").exists()).toBe(false);
    expect(wrapper.find("button[aria-label='New terminal for Base']").exists()).toBe(true);
  });

  it("offers a terminal on every workdir that has a directory, and on none that does not", () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              checkout({ id: "checkout:empty", isPrimary: true, branch: "main" }),
              {
                ...checkout({
                  id: "checkout:busy",
                  path: "/test-busy",
                  canonicalPath: "/test-busy",
                  isPrimary: false,
                  branch: "feature",
                }),
                sessions: [session("session:one", "zsh", "checkout:busy")],
              },
              {
                ...checkout({
                  id: "checkout:gone",
                  path: "/test-gone",
                  canonicalPath: "/test-gone",
                  isPrimary: false,
                  branch: "temporary",
                }),
                isMissing: true,
              },
            ],
          }),
        ],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });

    // The row is not gated on having terminals open, unlike the mockup: selecting a workdir no
    // longer opens one, so a workdir with nothing running still needs its way in. What does
    // hide it is a directory that is gone, which has no shell to run.
    expect(
      wrapper.findAll('button[aria-label^="New terminal for"]').map((row) => row.attributes("aria-label")),
    ).toEqual(["New terminal for main", "New terminal for feature"]);
    wrapper.unmount();
  });

  it("says in the click whether the row is advertising changes, so the panel can open the diff", async () => {
    mocks.getGitCheckoutDiffStats.mockResolvedValue({
      "checkout:changed": { additions: 12, deletions: 4 },
      "checkout:clean": { additions: 0, deletions: 0 },
    });
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              { ...checkout({ id: "checkout:changed", isPrimary: true, branch: "main" }) },
              {
                ...checkout({
                  id: "checkout:clean",
                  path: "/test-clean",
                  canonicalPath: "/test-clean",
                  isPrimary: false,
                  branch: "clean",
                }),
              },
            ],
          }),
        ],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });
    await flushPromises();

    const rows = wrapper.findAll(".workdir-group > .workdir-item > .workdir-row > .workdir-select");
    await rows[0]!.trigger("click");
    await rows[1]!.trigger("click");

    // The flag travels with the click, taken from the same counts the row paints, so the panel
    // opens the change set exactly when the row is advertising one.
    expect(wrapper.emitted("selectCheckout")).toEqual([
      ["checkout:changed", true],
      ["checkout:clean", false],
    ]);
    wrapper.unmount();
  });

  it("lists every terminal child, marks the active one and offers a close", async () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            id: "repo:activity",
            kind: "plain",
            name: "activity",
            root: "/activity",
            checkouts: [
              {
                id: "checkout:activity",
                repoId: "repo:activity",
                path: "/activity",
                canonicalPath: "/activity",
                isPrimary: true,
                changedFiles: 0,
                isMissing: false,
                sessions: [
                  {
                    id: "session:running",
                    type: "shell",
                    checkoutId: "checkout:activity",
                    name: "Running",
                    createdAt: "now",
                    status: "inactive",
                  },
                  {
                    id: "session:exited",
                    type: "shell",
                    checkoutId: "checkout:activity",
                    name: "Exited",
                    createdAt: "now",
                    status: "active",
                  },
                  {
                    id: "session:unknown",
                    type: "shell",
                    checkoutId: "checkout:activity",
                    name: "Unknown",
                    createdAt: "now",
                    status: "active",
                  },
                ],
              },
            ],
          }),
        ],
        activeCheckoutId: "checkout:activity",
        activeSessionId: "session:exited",
        isOpening: false,
        sessionRuntimeStatuses: { "session:exited": { state: "exited", exitCode: 1 } },
      },
    });

    const sessions = wrapper.findAll('button[aria-label^="Terminal session:"]');
    expect(sessions).toHaveLength(3);
    expect(sessions.map((session) => session.attributes("aria-label"))).toEqual([
      "Terminal session: Running",
      "Terminal session: Exited",
      "Terminal session: Unknown",
    ]);
    expect(sessions[1].attributes("aria-current")).toBe("page");
    // The mockup gives the icons exactly two colours: faint for everything, and the accent for
    // the active item. A third one for "live but not active" was a deviation and is gone, so the
    // only thing that sets an icon apart is whether its own row is the active one.
    expect(sessions.every((session) => !session.get(".workdir-status-icon").classes().includes("is-running"))).toBe(
      true,
    );
    expect(sessions[1].get(".workdir-status-icon").classes()).not.toContain("is-running");
    expect(wrapper.find('[role="img"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Concurrent");
    expect(wrapper.get(".workdir-item.has-active").get(".workdir-name").text()).toBe("Base");

    expect(wrapper.find('button[aria-label="Close terminal session: Exited"]').exists()).toBe(true);
    await wrapper.get('button[aria-label="Terminal session: Unknown"]').trigger("click");
    await wrapper.get('button[aria-label="Close terminal session: Exited"]').trigger("click");
    expect(wrapper.emitted("selectSession")).toEqual([["session:unknown"]]);
    expect(wrapper.emitted("closeSession")).toEqual([["session:exited"]]);
  });

  it("names the program in front of the shell, and nothing when the shell is in front", () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            id: "repo:apps",
            kind: "plain",
            name: "apps",
            root: "/apps",
            checkouts: [
              {
                id: "checkout:apps",
                repoId: "repo:apps",
                path: "/apps",
                canonicalPath: "/apps",
                isPrimary: true,
                changedFiles: 0,
                isMissing: false,
                sessions: [
                  session("session:agent", "zsh", "checkout:apps"),
                  session("session:editing", "zsh", "checkout:apps"),
                  session("session:idle", "zsh", "checkout:apps"),
                ],
              },
            ],
          }),
        ],
        activeCheckoutId: "checkout:apps",
        activeSessionId: null,
        isOpening: false,
        sessionRuntimeStatuses: {
          "session:agent": { state: "running", foregroundProcess: true, foregroundApp: "opencode" },
          "session:editing": { state: "running", foregroundProcess: true, foregroundApp: "nvim" },
          // Running with nothing in front: the resting state, which has no name to show.
          "session:idle": { state: "running", foregroundProcess: false },
        },
      },
    });

    // A row is named after what is running in it, and there is no second chip saying the same:
    // the idle terminal is back to the shell it was opened as, because nothing is in front of it.
    const names = wrapper.findAll(".workdir-child .workdir-name").map((row) => row.text());
    expect(names.slice(0, 3)).toEqual(["opencode", "nvim", "zsh"]);
    expect(wrapper.find(".app-chip").exists()).toBe(false);
    wrapper.unmount();
  });

  it("paints the active workdir's agent in OpenCode's own color, and spins only while it works", () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            id: "repo:agent",
            kind: "plain",
            name: "agent",
            root: "/agent",
            checkouts: [
              {
                ...checkout({ id: "checkout:agent" }),
                sessions: [session("session:one", "zsh", "checkout:agent")],
              },
            ],
          }),
        ],
        activeCheckoutId: "checkout:agent",
        activeSessionId: null,
        isOpening: false,
        agent: { label: "plan", color: "#FF966C", attention: "busy" },
        sessionRuntimeStatuses: {
          "session:one": { state: "running", foregroundProcess: true, foregroundApp: "opencode" },
        },
      },
    });

    const chip = wrapper.get(".agent-chip");
    expect(chip.text()).toBe("plan");
    // The color is the server's, so it is checked before it becomes a background.
    expect(chip.get(".agent-dot").attributes("style")).toContain("#FF966C");
    // Working is the only state with a spinner, and it survives a hover like an error does.
    expect(wrapper.find(".agent-spinner").exists()).toBe(true);
    expect(wrapper.get(".workdir-child .workdir-meta").classes()).toContain("workdir-meta-pinned");
    expect(chip.attributes("title")).toBe("OpenCode agent: plan");
    wrapper.unmount();
  });

  it("claims the agent only for the terminal OpenCode is the one running in it", () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              {
                ...checkout({ id: "checkout:agent" }),
                sessions: [
                  session("session:agent", "zsh", "checkout:agent"),
                  session("session:editing", "zsh", "checkout:agent"),
                  session("session:idle", "zsh", "checkout:agent"),
                ],
              },
            ],
          }),
        ],
        activeCheckoutId: "checkout:agent",
        activeSessionId: null,
        isOpening: false,
        agent: { label: "coder", color: null, attention: "none" },
        sessionRuntimeStatuses: {
          "session:agent": { state: "running", foregroundProcess: true, foregroundApp: "opencode" },
          "session:editing": { state: "running", foregroundProcess: true, foregroundApp: "nvim" },
          "session:idle": { state: "running", foregroundProcess: false },
        },
      },
    });

    // The workdir has an agent behind it and exactly one of its three terminals is running it. A
    // Neovim row and an idle row naming one would be a claim about a process that is not in front.
    const chips = wrapper.findAll(".workdir-child .agent-chip");
    expect(chips).toHaveLength(1);
    expect(chips[0]!.text()).toBe("coder");
    wrapper.unmount();
  });

  it("leaves a quiet agent unpinned, and names no agent on a workdir with no server", async () => {
    const quiet = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [{ ...checkout({ id: "checkout:agent" }), sessions: [session("s", "zsh", "checkout:agent")] }],
          }),
        ],
        activeCheckoutId: "checkout:agent",
        activeSessionId: null,
        isOpening: false,
        agent: { label: "coder", color: null, attention: "none" },
        sessionRuntimeStatuses: {
          s: { state: "running", foregroundProcess: true, foregroundApp: "opencode" },
        },
      },
    });

    // A quiet agent is still named, in the accent it has no color of its own for.
    const chip = quiet.get(".agent-chip");
    expect(chip.text()).toBe("coder");
    expect(chip.get(".agent-dot").attributes("style")).toContain("--marvis-accent");
    expect(quiet.find(".agent-spinner").exists()).toBe(false);
    expect(quiet.get(".workdir-child .workdir-meta").classes()).not.toContain("workdir-meta-pinned");
    quiet.unmount();

    // The agent belongs to a checkout, so a row under a workdir that is not the active one
    // has nothing to say: only the active checkout has a server behind it.
    const elsewhere = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              { ...checkout({ id: "checkout:one" }), sessions: [session("a", "zsh", "checkout:one")] },
              {
                ...checkout({
                  id: "checkout:two",
                  path: "/two",
                  canonicalPath: "/two",
                  isPrimary: false,
                  branch: "two",
                }),
                sessions: [session("b", "zsh", "checkout:two")],
              },
            ],
          }),
        ],
        activeCheckoutId: "checkout:two",
        activeSessionId: null,
        isOpening: false,
        agent: { label: "plan", color: "#FF966C", attention: "busy" },
        sessionRuntimeStatuses: {
          a: { state: "running", foregroundProcess: true, foregroundApp: "opencode" },
          b: { state: "running", foregroundProcess: true, foregroundApp: "opencode" },
        },
      },
    });
    await flushPromises();
    // One chip, and it hangs off the active workdir: the other checkout has no server behind it.
    expect(elsewhere.findAll(".agent-chip")).toHaveLength(1);
    const [firstItems, activeItems] = elsewhere.findAll(".workdir-items");
    expect(firstItems!.find(".agent-chip").exists()).toBe(false);
    expect(activeItems!.find(".agent-chip").exists()).toBe(true);
    elsewhere.unmount();
  });

  it("renames a session in place, and only when the name actually changed", async () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            id: "repo:rename",
            kind: "plain",
            name: "rename",
            root: "/rename",
            checkouts: [
              {
                ...checkout({ id: "checkout:rename" }),
                sessions: [session("session:one", "zsh", "checkout:rename")],
              },
            ],
          }),
        ],
        activeCheckoutId: "checkout:rename",
        activeSessionId: null,
        isOpening: false,
      },
      // In the document, because the field takes the focus and that is half of what a rename
      // has to do.
      attachTo: document.body,
    });

    const row = () => wrapper.get('button[aria-label="Terminal session: zsh"]');
    await row().trigger("dblclick");
    // The field appears and takes the focus on the tick after the click, not the same one.
    await flushPromises();

    // The button is replaced by the field, not wrapped in it: a control inside a control
    // cannot be focused or announced on its own.
    expect(wrapper.find(".workdir-rename").exists()).toBe(true);
    const field = wrapper.get<HTMLInputElement>(".workdir-rename");
    expect(field.element.value).toBe("zsh");
    expect(document.activeElement).toBe(field.element);

    // Enter keeps what was typed. The spaces around it are not part of a name, and the backend
    // trims them again on its own side: the database decides, not this row.
    await field.setValue("  build logs  ");
    await field.trigger("keydown.enter");
    expect(wrapper.emitted("renameSession")).toEqual([["session:one", "build logs"]]);
    expect(wrapper.find(".workdir-rename").exists()).toBe(false);

    // Escape is not a rename: the field closes and nothing is emitted.
    await row().trigger("dblclick");
    await flushPromises();
    await wrapper.get(".workdir-rename").setValue("discarded");
    await wrapper.get(".workdir-rename").trigger("keydown.esc");
    expect(wrapper.find(".workdir-rename").exists()).toBe(false);
    expect(wrapper.emitted("renameSession")).toHaveLength(1);

    // Nor is retyping the name it already had.
    await row().trigger("dblclick");
    await flushPromises();
    await wrapper.get(".workdir-rename").trigger("keydown.enter");
    expect(wrapper.emitted("renameSession")).toHaveLength(1);
    wrapper.unmount();
  });

  it("keeps a long workdir title ellipsizable under the hover gutter", () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              {
                id: "checkout:long",
                repoId: "repo:test",
                path: "/test-feature",
                canonicalPath: "/test-feature",
                isPrimary: false,
                branch: "bug/13104984920-timeline-element-boundaries",
                changedFiles: 0,
                isMissing: false,
                sessions: [],
              },
            ],
          }),
        ],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });

    const name = wrapper.get(".workdir-name");
    expect(name.text()).toBe("bug/13104984920-timeline-element-boundaries");
    expect(name.element.className).toBe("workdir-name");
    expect(wrapper.get(".workdir-title").element.className).toBe("workdir-title");
    expect(wrapper.get(".workdir-item .workdir-select").element.className).toBe("workdir-select");
    expect(wrapper.find(".workdir-meta").exists()).toBe(true);
  });

  it("cuts a branch name at the ticket, so the part that tells branches apart survives", () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              checkout({
                id: "checkout:feature",
                path: "/test-feature",
                isPrimary: false,
                branch: "feat/12529440962-titan-hero-homepage-banner-imagen",
              }),
            ],
          }),
        ],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });

    // The ticket prefix is the half every branch of the repo shares, so it is the half the
    // ellipsis is allowed to reach.
    expect(wrapper.get(".workdir-name-head").text()).toBe("feat/12529440962-");
    expect(wrapper.get(".workdir-name-tail").text()).toBe("titan-hero-homepage-banner-imagen");
    // The name is still one name: the halves are a way to draw it, and nothing sits between them.
    expect(wrapper.get(".workdir-name").text()).toBe("feat/12529440962-titan-hero-homepage-banner-imagen");
  });

  it("draws a name with no ticket in it whole, and says the same on hover", () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [checkout({ id: "checkout:detached", isPrimary: false, branch: undefined })],
          }),
        ],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });

    // No ticket means no head to keep, so the name is one piece rather than an empty one beside
    // itself.
    expect(wrapper.find(".workdir-name-head").exists()).toBe(false);
    expect(wrapper.get(".workdir-name-tail").text()).toBe("/test");
    // A checkout with no branch to name it has only its path to say, hover included.
    expect(wrapper.get(".workdir-select").attributes("title")).toBe("/test");
  });

  it("shows every checkout's own line counts, not only the active one's", async () => {
    mocks.getGitCheckoutDiffStats.mockResolvedValue({
      "checkout:primary": { additions: 12, deletions: 4 },
      "checkout:feature": { additions: 0, deletions: 7 },
    });
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              {
                id: "checkout:primary",
                repoId: "repo:test",
                path: "/test",
                canonicalPath: "/test",
                isPrimary: true,
                branch: "main",
                changedFiles: 0,
                isMissing: false,
                sessions: [],
              },
              {
                id: "checkout:feature",
                repoId: "repo:test",
                path: "/test-feature",
                canonicalPath: "/test-feature",
                isPrimary: false,
                branch: "feature",
                changedFiles: 0,
                isMissing: false,
                sessions: [],
              },
            ],
          }),
        ],
        activeCheckoutId: "checkout:feature",
        activeSessionId: null,
        isOpening: false,
      },
    });
    await flushPromises();

    // One call covers the sidebar: the inactive checkout is counted without being visited.
    expect(mocks.getGitCheckoutDiffStats).toHaveBeenCalledTimes(1);
    const [base, feature] = wrapper.findAll(".workdir-item .workdir-meta");
    expect(base.get(".diff-add").text()).toBe("+12");
    expect(base.get(".diff-del").text()).toBe("-4");
    // A zero addition has nothing to draw, and the deletion stands on its own.
    expect(feature.find(".diff-add").exists()).toBe(false);
    expect(feature.get(".diff-del").text()).toBe("-7");
    wrapper.unmount();
  });

  it("draws no counts for a checkout Git cannot answer for", async () => {
    mocks.getGitCheckoutDiffStats.mockResolvedValue({ "checkout:primary": { additions: 0, deletions: 0 } });
    const wrapper = mount(Sidebar, {
      props: {
        repos: [repo({ checkouts: [{ ...checkout(), id: "checkout:primary" }] })],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });
    await flushPromises();

    // An absent checkout is not a clean bill of health, and neither is a real zero.
    expect(wrapper.get(".workdir-meta").text()).toBe("");
    wrapper.unmount();
  });

  it("names a missing directory in the row that owns it, in place of its counts", async () => {
    mocks.getGitCheckoutDiffStats.mockResolvedValue({
      "checkout:primary": { additions: 12, deletions: 4 },
      "checkout:gone": { additions: 3, deletions: 1 },
    });
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              checkout({ id: "checkout:primary" }),
              checkout({ id: "checkout:gone", path: "/test-gone", isMissing: true }),
            ],
          }),
        ],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });
    await flushPromises();

    const [base, gone] = wrapper.findAll(".workdir-item .workdir-meta");
    expect(base.text()).toBe("+12-4");
    // E.4: a failure of one workdir is written in that row, and it takes the slot the counts
    // would have used, so a row never shows a failure next to figures it cannot have.
    expect(gone.text()).toBe("Directory missing");
    expect(gone.classes()).toContain("workdir-meta-error");
    expect(gone.find(".diff-add").exists()).toBe(false);
    wrapper.unmount();
  });

  it("disables a missing directory and leaves closing it as its only action", async () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [
          repo({
            checkouts: [
              {
                ...checkout({
                  id: "checkout:feature",
                  path: "/test-feature",
                  canonicalPath: "/test-feature",
                  isPrimary: false,
                  branch: "feature",
                }),
                sessions: [session("session:live", "Live", "checkout:feature")],
              },
              {
                ...checkout({
                  id: "checkout:gone",
                  path: "/test-gone",
                  canonicalPath: "/test-gone",
                  isPrimary: false,
                  branch: "temporary",
                  isMissing: true,
                }),
                sessions: [session("session:stale", "Stale", "checkout:gone")],
              },
            ],
          }),
        ],
        activeCheckoutId: "checkout:feature",
        activeSessionId: null,
        isOpening: false,
      },
    });

    // The rows themselves, not the session children hanging under them.
    const [live, gone] = wrapper.findAll(".workdir-group > .workdir-item > .workdir-row > .workdir-select");

    expect(live!.attributes("aria-disabled")).toBeUndefined();
    expect(gone!.attributes("aria-disabled")).toBe("true");
    // The reason the row is disabled stays in the row: it is the whole message now.
    expect(gone!.text()).toContain("Directory missing");
    // Nothing behind a directory that is gone is offered: none of its sessions, no shell.
    expect(
      wrapper.findAll('button[aria-label^="Terminal session:"]').map((row) => row.attributes("aria-label")),
    ).toEqual(["Terminal session: Live"]);
    expect(
      wrapper.findAll('button[aria-label^="New terminal for"]').map((row) => row.attributes("aria-label")),
    ).toEqual(["New terminal for feature"]);
    // A live worktree offers one way out, and it asks before it acts: archiving and deleting
    // are both answers to the same question, so the cross opens the dialog that holds them.
    expect(wrapper.find('button[aria-label="Remove or archive worktree feature"]').exists()).toBe(true);
    expect(wrapper.find('button[aria-label="Remove or archive worktree temporary"]').exists()).toBe(false);
    // One action per thing the row can do: the live worktree's own one, and the missing
    // one's close.
    expect(wrapper.findAll(".workdir-group > .workdir-item .workdir-actions button")).toHaveLength(2);
    expect(wrapper.get('button[aria-label="Remove or archive worktree feature"]').attributes("title")).toBe(
      "Remove or archive worktree",
    );
    const close = wrapper.get('button[aria-label="Close missing checkout: temporary"]');
    expect(close.attributes("title")).toBe("Remove from list");

    await gone!.trigger("click");
    await live!.trigger("click");
    await close.trigger("click");
    await wrapper.get('button[aria-label="Remove or archive worktree feature"]').trigger("click");

    expect(wrapper.emitted("selectCheckout")).toEqual([["checkout:feature", false]]);
    expect(wrapper.emitted("closeMissing")).toEqual([["checkout:gone"]]);
    expect(wrapper.emitted("removeWorktree")).toEqual([["checkout:feature"]]);
    wrapper.unmount();
  });

  it("hangs the restore action on the repo root, and only while it has something to restore", async () => {
    const gitdir = { ...checkout({ id: "checkout:primary" }), branch: "main" };
    const worktree = {
      ...checkout({
        id: "checkout:feature",
        path: "/test-feature",
        canonicalPath: "/test-feature",
        isPrimary: false,
        branch: "feature",
      }),
    };
    const mounted = (archivedWorktrees: ArchivedCheckout[]) =>
      mount(Sidebar, {
        props: {
          repos: [repo({ checkouts: [gitdir, worktree] })],
          activeCheckoutId: null,
          activeSessionId: null,
          isOpening: false,
          archivedWorktrees,
        },
      });

    // Nothing archived, nothing to offer: a button that can only answer "nothing archived"
    // is a button that teaches the row it sits on means nothing.
    const empty = mounted([]);
    expect(empty.find('button[aria-label^="Restore"]').exists()).toBe(false);
    empty.unmount();

    const wrapper = mounted([{ id: "checkout:gone", repoId: "repo:test", path: "/test-gone", branch: "temporary" }]);
    const restore = wrapper.get('button[aria-label="Restore 1 archived worktree from main"]');
    expect(restore.attributes("title")).toBe("Restore archived worktrees");
    // The root reserves room for its own two actions plus the third it only sometimes has,
    // so a long branch name cannot end up underneath them.
    expect(wrapper.get(".workdir-item").classes()).toContain("has-three-actions");

    await restore.trigger("click");

    expect(wrapper.emitted("restoreArchived")).toEqual([["repo:test"]]);
    wrapper.unmount();
  });

  it("counts the plural in what it says, so the row never promises one of many", () => {
    const wrapper = mount(Sidebar, {
      props: {
        repos: [repo({ checkouts: [checkout({ id: "checkout:primary", branch: "main" })] })],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
        archivedWorktrees: [
          { id: "checkout:a", repoId: "repo:test", path: "/test-a", branch: "a" },
          { id: "checkout:b", repoId: "repo:test", path: "/test-b", branch: "b" },
        ],
      },
    });

    expect(wrapper.find('button[aria-label="Restore 2 archived worktrees from main"]').exists()).toBe(true);
    wrapper.unmount();
  });
});
