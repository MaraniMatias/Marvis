// @vitest-environment happy-dom
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import type { Repo } from "../domain/workspace";
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

describe("Sidebar workdir rows", () => {
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
        activityByCheckout: { "checkout:activity": ["Terminal · Running", "Recent file writes"] },
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
});
