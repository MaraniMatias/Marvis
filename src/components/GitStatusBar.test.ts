// @vitest-environment happy-dom
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import type { GitStatus } from "../domain/git";
import type { Checkout, Repo } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import GitStatusBar from "./GitStatusBar.vue";

function checkout(id: string, isMissing = false): Checkout {
  return {
    id,
    repoId: "repo:test",
    path: "/test",
    canonicalPath: "/test",
    isPrimary: true,
    changedFiles: 0,
    isMissing,
    sessions: [],
  };
}

const gitRepo: Repo = {
  id: "repo:test",
  kind: "git",
  name: "test",
  root: "/test",
  defaultBranch: "trunk",
  checkouts: [],
  createdAt: "now",
  lastOpenedAt: "now",
};

function snapshot(status: GitStatus | null = null, overrides: Partial<ActiveGitSnapshot> = {}): ActiveGitSnapshot {
  return {
    checkoutId: "checkout:first",
    status,
    viewedPaths: [],
    loading: false,
    statusState: status ? "ready" : "error",
    statusError: status ? "" : "Git status unavailable",
    changesStatusError: "",
    viewedError: "",
    watchError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
    markViewed: async () => undefined,
    ...overrides,
  };
}

describe("GitStatusBar", () => {
  it("renders the shared status and viewed snapshot", () => {
    const status: GitStatus = {
      branch: "trunk",
      defaultBranch: "trunk",
      aheadCount: 1,
      files: [{ path: "file.txt", status: "M" }],
    };
    const wrapper = mount(GitStatusBar, {
      props: {
        checkout: checkout("checkout:first"),
        repo: gitRepo,
        gitSnapshot: snapshot(status, { viewedPaths: ["file.txt"] }),
      },
    });

    expect(wrapper.text()).toContain("trunk");
    expect(wrapper.text()).toContain("1 changed");
    expect(wrapper.text()).toContain("1 commit ahead");
    expect(wrapper.text()).toContain("1/1 viewed");
  });

  it("shows plain and missing states without a Git snapshot", async () => {
    const plainRepo = { ...gitRepo, kind: "plain" as const, defaultBranch: undefined };
    const wrapper = mount(GitStatusBar, {
      props: { checkout: checkout("checkout:plain"), repo: plainRepo, gitSnapshot: snapshot() },
    });
    expect(wrapper.text()).toContain("Plain directory");

    await wrapper.setProps({ checkout: checkout("checkout:missing", true), repo: gitRepo });
    expect(wrapper.text()).toContain("Directory missing");
    wrapper.unmount();
  });

  it("shows an informational concurrent activity signal without blocking", () => {
    const wrapper = mount(GitStatusBar, {
      props: {
        checkout: checkout("checkout:activity"),
        repo: gitRepo,
        gitSnapshot: snapshot(),
        concurrentActors: ["Neovim · nvim", "Recent file writes"],
      },
    });

    expect(wrapper.get('[aria-label="Concurrent activity"]').text()).toContain("Neovim · nvim");
    expect(wrapper.get('[aria-label="Concurrent activity"]').attributes("title")).toContain(
      "External editors and agents are not tracked",
    );
    expect(wrapper.find("button").exists()).toBe(false);
    wrapper.unmount();
  });
});
