// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout, Repo, Session } from "../domain/workspace";

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

  it("names the repo root Base with its branch and keeps worktree actions apart", async () => {
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
    // The repo root is "Base" with the branch aside; a worktree is named by its branch.
    expect(wrapper.get(".workdir-item .workdir-name").text()).toBe("Base");
    expect(wrapper.get(".workdir-branch").text()).toBe("·main");
    expect(wrapper.get(".workdir-item.active .workdir-select").attributes("title")).toBe("/test-feature");

    expect(wrapper.find('button[aria-label="Add worktree from main"]').exists()).toBe(true);
    expect(wrapper.find('button[aria-label="Remove worktree Base"]').exists()).toBe(false);
    expect(wrapper.find('button[aria-label="Remove worktree feature"]').exists()).toBe(true);
    expect(wrapper.find('button[aria-label="New terminal for Base"]').exists()).toBe(true);
    expect(wrapper.text()).not.toContain("Plain");

    await wrapper.get(".workdir-item.active .workdir-select").trigger("click");
    await wrapper.get('button[aria-label="Add worktree from main"]').trigger("click");
    await wrapper.get('button[aria-label="Remove worktree feature"]').trigger("click");
    await wrapper.get('button[aria-label="New terminal for Base"]').trigger("click");

    expect(wrapper.emitted("selectCheckout")).toEqual([["checkout:feature"]]);
    expect(wrapper.emitted("createWorktree")).toEqual([["checkout:primary"]]);
    expect(wrapper.emitted("removeWorktree")).toEqual([["checkout:feature"]]);
    expect(wrapper.emitted("newTerminal")).toEqual([["checkout:primary"]]);
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

    expect(wrapper.find(".workdir-actions button").exists()).toBe(false);
    expect(wrapper.find("button[aria-label^='Add worktree']").exists()).toBe(false);
    expect(wrapper.find("button[aria-label^='Remove worktree']").exists()).toBe(false);
    expect(wrapper.find("button[aria-label='New terminal for Base']").exists()).toBe(true);
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
    // The active item takes the accent icon, a live one a step brighter than an exited one.
    expect(sessions[1].get(".workdir-status-icon").classes()).not.toContain("is-running");
    expect(sessions[2].get(".workdir-status-icon").classes()).toContain("is-running");
    expect(wrapper.find('[role="img"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Concurrent");
    expect(wrapper.get(".workdir-item.has-active").get(".workdir-name").text()).toBe("Base");

    expect(wrapper.find('button[aria-label="Close terminal session: Exited"]').exists()).toBe(true);
    await wrapper.get('button[aria-label="Terminal session: Unknown"]').trigger("click");
    await wrapper.get('button[aria-label="Close terminal session: Exited"]').trigger("click");
    expect(wrapper.emitted("selectSession")).toEqual([["session:unknown"]]);
    expect(wrapper.emitted("closeSession")).toEqual([["session:exited"]]);
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
    // A live worktree keeps the action that deletes it from disk, and no closing one.
    expect(wrapper.find('button[aria-label="Remove worktree feature"]').exists()).toBe(true);
    expect(wrapper.find('button[aria-label="Remove worktree temporary"]').exists()).toBe(false);
    // One action per workdir row: the trash of the live worktree, and the missing one's close.
    expect(wrapper.findAll(".workdir-group > .workdir-item .workdir-actions button")).toHaveLength(2);
    const close = wrapper.get('button[aria-label="Close missing checkout: temporary"]');
    expect(close.attributes("title")).toBe("Remove from list");

    await gone!.trigger("click");
    await live!.trigger("click");
    await close.trigger("click");

    expect(wrapper.emitted("selectCheckout")).toEqual([["checkout:feature"]]);
    expect(wrapper.emitted("closeMissing")).toEqual([["checkout:gone"]]);
    wrapper.unmount();
  });
});
