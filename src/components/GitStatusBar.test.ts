// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout, Repo } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  getGitStatus: vi.fn(),
  getGitViewedFiles: vi.fn(),
  watchGitCheckout: vi.fn(),
  unwatchGitCheckout: vi.fn(),
  listen: vi.fn(),
  onStatusChanged: null as ((event: { payload: string }) => void) | null,
  onViewedChanged: null as ((event: { payload: string }) => void) | null,
}));

vi.mock("../lib/ipc", () => ({
  getGitStatus: mocks.getGitStatus,
  getGitViewedFiles: mocks.getGitViewedFiles,
  watchGitCheckout: mocks.watchGitCheckout,
  unwatchGitCheckout: mocks.unwatchGitCheckout,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

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

describe("GitStatusBar", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.onStatusChanged = null;
    mocks.onViewedChanged = null;
    mocks.listen.mockImplementation(async (event: string, handler: (event: { payload: string }) => void) => {
      if (event === "git-viewed-changed") mocks.onViewedChanged = handler;
      else mocks.onStatusChanged = handler;
      return vi.fn();
    });
    mocks.watchGitCheckout.mockResolvedValue(undefined);
    mocks.unwatchGitCheckout.mockResolvedValue(undefined);
    mocks.getGitStatus.mockImplementation(async (checkoutId: string) => ({
      branch: checkoutId.endsWith("second") ? "feature" : "trunk",
      defaultBranch: "trunk",
      aheadCount: checkoutId.endsWith("second") ? 2 : 1,
      files: Array.from({ length: checkoutId.endsWith("second") ? 3 : 1 }, (_, index) => ({
        path: `file-${index}.txt`,
        status: "M",
      })),
    }));
    mocks.getGitViewedFiles.mockImplementation(async (checkoutId: string) =>
      checkoutId.endsWith("second") ? ["file-0.txt", "file-1.txt"] : ["file-0.txt"],
    );
  });

  it("loads on selection and refreshes the selected checkout from watcher events", async () => {
    const wrapper = mount(GitStatusBar, { props: { checkout: checkout("checkout:first"), repo: gitRepo } });
    await flushPromises();
    expect(wrapper.text()).toContain("trunk");
    expect(wrapper.text()).toContain("1 changed");
    expect(wrapper.text()).toContain("1 commit ahead");
    expect(wrapper.text()).toContain("1/1 viewed");
    expect(mocks.watchGitCheckout).toHaveBeenCalledWith("checkout:first");

    mocks.onStatusChanged?.({ payload: "checkout:first" });
    await flushPromises();
    expect(mocks.getGitStatus).toHaveBeenCalledTimes(2);

    await wrapper.setProps({ checkout: checkout("checkout:second") });
    await flushPromises();
    expect(mocks.unwatchGitCheckout).toHaveBeenCalledWith("checkout:first");
    expect(mocks.watchGitCheckout).toHaveBeenLastCalledWith("checkout:second");
    expect(wrapper.text()).toContain("feature");
    expect(wrapper.text()).toContain("3 changed");
    expect(wrapper.text()).toContain("2 commits ahead");
    expect(wrapper.text()).toContain("2/3 viewed");

    mocks.onViewedChanged?.({ payload: "checkout:second" });
    await flushPromises();
    expect(mocks.getGitViewedFiles).toHaveBeenCalledTimes(4);
    wrapper.unmount();
  });

  it("shows plain and missing states without running Git", async () => {
    const plainRepo = { ...gitRepo, kind: "plain" as const, defaultBranch: undefined };
    const wrapper = mount(GitStatusBar, { props: { checkout: checkout("checkout:plain"), repo: plainRepo } });
    await flushPromises();
    expect(wrapper.text()).toContain("Plain directory");
    expect(mocks.getGitStatus).not.toHaveBeenCalled();
    expect(mocks.watchGitCheckout).not.toHaveBeenCalled();

    await wrapper.setProps({ checkout: checkout("checkout:missing", true), repo: gitRepo });
    await flushPromises();
    expect(wrapper.text()).toContain("Directory missing");
    expect(mocks.getGitStatus).not.toHaveBeenCalled();
    expect(mocks.watchGitCheckout).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("shows an informational concurrent activity signal without blocking", () => {
    const wrapper = mount(GitStatusBar, {
      props: {
        checkout: checkout("checkout:activity"),
        repo: gitRepo,
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
