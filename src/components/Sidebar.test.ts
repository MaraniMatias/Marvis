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
      props: { repos: [repo], activeCheckoutId: null, activeSessionId: null, isOpening: false },
    });

    expect(wrapper.get('button[aria-label="Open directory"]')).toBeDefined();
    expect(wrapper.text()).toContain("Open directory");
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

  it("offers create from each available Git checkout and removal only for worktrees", async () => {
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
          branch: "trunk",
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
      props: { repos: [repo], activeCheckoutId: null, activeSessionId: null, isOpening: false },
    });

    await wrapper.get('button[aria-label="Create worktree from trunk"]').trigger("click");
    await wrapper.get('button[aria-label="Create worktree from feature"]').trigger("click");
    await wrapper.get('button[aria-label="Remove worktree feature"]').trigger("click");

    expect(wrapper.emitted("createWorktree")).toEqual([["checkout:primary"], ["checkout:feature"]]);
    expect(wrapper.emitted("removeWorktree")).toEqual([["checkout:feature"]]);
  });

  it("shows only runtime-verified running and exited dots", () => {
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
        activeSessionId: null,
        isOpening: false,
        sessionRuntimeStatuses: {
          "session:running": { state: "running", foregroundProcess: true },
          "session:exited": { state: "exited", exitCode: 1, foregroundProcess: false },
        },
        activityByCheckout: { "checkout:activity": ["Terminal · Running", "Recent file writes"] },
      },
    });

    expect(wrapper.find('[aria-label="Session running"]').exists()).toBe(true);
    expect(wrapper.find('[aria-label="Session exited"]').exists()).toBe(true);
    expect(wrapper.findAll('[aria-label^="Session "]')).toHaveLength(2);
    expect(wrapper.get('[aria-label="Concurrent activity: Terminal · Running, Recent file writes"]').text()).toBe(
      "Concurrent",
    );
  });
});
