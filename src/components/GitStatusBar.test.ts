// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout, Repo } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  getGitStatus: vi.fn(),
  watchGitCheckout: vi.fn(),
  unwatchGitCheckout: vi.fn(),
  listen: vi.fn(),
  onStatusChanged: null as ((event: { payload: string }) => void) | null,
}));

vi.mock("../lib/ipc", () => ({
  getGitStatus: mocks.getGitStatus,
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
    mocks.listen.mockImplementation(async (_event: string, handler: (event: { payload: string }) => void) => {
      mocks.onStatusChanged = handler;
      return vi.fn();
    });
    mocks.watchGitCheckout.mockResolvedValue(undefined);
    mocks.unwatchGitCheckout.mockResolvedValue(undefined);
    mocks.getGitStatus.mockImplementation(async (checkoutId: string) => ({
      branch: checkoutId.endsWith("second") ? "feature" : "trunk",
      defaultBranch: "trunk",
      aheadCount: 0,
      files: Array.from({ length: checkoutId.endsWith("second") ? 3 : 1 }, (_, index) => ({
        path: `file-${index}.txt`,
        status: "M",
      })),
    }));
  });

  it("loads on selection and refreshes the selected checkout from watcher events", async () => {
    const wrapper = mount(GitStatusBar, { props: { checkout: checkout("checkout:first"), repo: gitRepo } });
    await flushPromises();
    expect(wrapper.text()).toContain("trunk");
    expect(wrapper.text()).toContain("1 changed");
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
});
