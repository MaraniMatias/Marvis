// @vitest-environment happy-dom
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import type { Repo } from "../domain/workspace";
import Sidebar from "./Sidebar.vue";

describe("Sidebar actions and checkout states", () => {
  it("offers Open directory and marks missing checkouts", async () => {
    const repo: Repo = {
      id: "repo:test",
      kind: "git",
      name: "test",
      root: "/test",
      checkouts: [
        {
          id: "checkout:gone",
          repoId: "repo:test",
          path: "/test/gone",
          canonicalPath: "/test/gone",
          isPrimary: false,
          changedFiles: 0,
          isMissing: true,
          sessions: [],
        },
      ],
      createdAt: "now",
      lastOpenedAt: "now",
    };
    const wrapper = mount(Sidebar, {
      props: {
        repos: [repo],
        activeCheckoutId: null,
        activeSessionId: null,
        isOpening: false,
      },
    });

    expect(wrapper.find("h2").exists()).toBe(false);
    expect(wrapper.get('input[aria-label="Search repositories"]').element.getAttribute("placeholder")).toBe(
      "Filter repositories…",
    );
    expect(wrapper.get('button[aria-label="Open directory"]')).toBeDefined();
    expect(wrapper.get('button[aria-label="Open directory"]').attributes("title")).toBe("Open directory");
    expect(wrapper.text()).toContain("Missing");
    expect(wrapper.find('button[aria-label="Locate /test/gone"]').exists()).toBe(true);
    expect(wrapper.find('button[aria-label="Close /test/gone"]').exists()).toBe(true);
    await wrapper.get('button[aria-label="Locate /test/gone"]').trigger("click");
    await wrapper.get('button[aria-label="Close /test/gone"]').trigger("click");
    expect(wrapper.emitted("locateMissing")).toEqual([["checkout:gone"]]);
    expect(wrapper.emitted("closeMissing")).toEqual([["checkout:gone"]]);
    await wrapper.get('button[aria-label="Open directory"]').trigger("click");
    expect(wrapper.emitted("openFolder")).toHaveLength(1);
  });

  it("offers create at the repository row and removal only for non-primary worktrees", async () => {
    const repo: Repo = {
      id: "repo:test",
      kind: "git",
      name: "test",
      root: "/test",
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
      createdAt: "now",
      lastOpenedAt: "now",
    };
    const wrapper = mount(Sidebar, {
      props: {
        repos: [repo],
        activeCheckoutId: "checkout:feature",
        activeSessionId: null,
        isOpening: false,
      },
    });

    expect(wrapper.find('button[aria-label="Create worktree for test"]').exists()).toBe(true);
    expect(wrapper.find('button[aria-label="Create worktree from trunk"]').exists()).toBe(false);
    expect(wrapper.find('button[aria-label="Create worktree from feature"]').exists()).toBe(false);
    expect(wrapper.find('button[aria-label="Remove worktree main"]').exists()).toBe(false);
    expect(wrapper.find('button[aria-label="New terminal for Primary · main"]').exists()).toBe(true);
    expect(wrapper.find('button[aria-label="Remove worktree feature"]').exists()).toBe(true);
    expect(wrapper.text()).not.toContain("Marvis");
    await wrapper.get('button[aria-label="New terminal for Primary · main"]').trigger("click");
    await wrapper.get('button[aria-label="Create worktree for test"]').trigger("click");
    await wrapper.get('button[aria-label="Remove worktree feature"]').trigger("click");

    expect(wrapper.emitted("newTerminal")).toEqual([["checkout:primary"]]);
    expect(wrapper.emitted("createWorktree")).toEqual([["checkout:primary"]]);
    expect(wrapper.emitted("removeWorktree")).toEqual([["checkout:feature"]]);
  });

  it("lists every terminal child with runtime status and close actions", async () => {
    const repo: Repo = {
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
      createdAt: "now",
      lastOpenedAt: "now",
    };
    const wrapper = mount(Sidebar, {
      props: {
        repos: [repo],
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
    expect(sessions[1].get('[role="img"]').attributes("aria-label")).toBe("Session exited");
    expect(wrapper.find('button[aria-label="Close terminal session: Exited"]').exists()).toBe(true);
    await wrapper.setProps({ activeCheckoutId: null, activeSessionId: null });
    await wrapper.get('button[aria-label="Terminal session: Unknown"]').trigger("click");
    await wrapper.get('button[aria-label="Close terminal session: Exited"]').trigger("click");
    expect(wrapper.emitted("selectSession")).toEqual([["session:unknown"]]);
    expect(wrapper.emitted("closeSession")).toEqual([["session:exited"]]);
    expect(wrapper.get('[aria-label="Concurrent activity: Terminal · Running, Recent file writes"]').text()).toBe(
      "Concurrent",
    );
    expect(wrapper.text()).toContain("Running");
  });
});
