// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout } from "../domain/workspace";

const mocks = vi.hoisted(() => ({
  getGitDiff: vi.fn(),
  getGitStatus: vi.fn(),
  listen: vi.fn(),
  onStatusChanged: null as ((event: { payload: string }) => void) | null,
}));

vi.mock("../lib/ipc", () => ({
  getGitDiff: mocks.getGitDiff,
  getGitStatus: mocks.getGitStatus,
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

vi.mock("@git-diff-view/vue", () => ({
  DiffFile: class {
    initTheme() {}
    init() {}
    buildUnifiedDiffLines() {}
  },
  DiffModeEnum: { Unified: 4 },
  DiffView: { template: "<div />" },
}));

import ChangesPane from "./ChangesPane.vue";

function checkout(): Checkout {
  return {
    id: "checkout:repo",
    repoId: "repo:repo",
    path: "/repo",
    canonicalPath: "/repo",
    isPrimary: true,
    changedFiles: 0,
    isMissing: false,
    sessions: [],
  };
}

describe("ChangesPane", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.onStatusChanged = null;
    mocks.listen.mockImplementation(async (_name: string, handler: (event: { payload: string }) => void) => {
      mocks.onStatusChanged = handler;
      return vi.fn();
    });
    mocks.getGitStatus.mockResolvedValue({
      branch: "feature",
      defaultBranch: "trunk",
      aheadCount: 1,
      files: [
        { path: "text.txt", status: "M" },
        { path: "image.bin", status: "A" },
      ],
    });
    mocks.getGitDiff.mockImplementation(async (_checkoutId: string, path: string) => ({
      path,
      patch: "",
      isBinary: path === "image.bin",
    }));
  });

  it("loads file diffs on demand and refreshes status and the selected diff from watcher events", async () => {
    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout(), defaultBranch: "trunk" },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("vs trunk · 2 changed");
    expect(mocks.getGitDiff).not.toHaveBeenCalled();

    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(mocks.getGitDiff).toHaveBeenCalledTimes(1);

    mocks.onStatusChanged?.({ payload: "checkout:repo" });
    await flushPromises();
    expect(mocks.getGitStatus).toHaveBeenCalledTimes(2);
    expect(mocks.getGitDiff).toHaveBeenCalledTimes(2);

    await wrapper.get("button:nth-of-type(2)").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("Binary file; text diff is unavailable.");

    wrapper.unmount();
  });

  it("delegates an unknown default branch to the existing choice flow", async () => {
    mocks.getGitStatus.mockRejectedValueOnce({
      code: "default_branch_unknown",
      message: "Choose a default branch",
    });
    const wrapper = mount(ChangesPane, {
      props: { checkout: checkout() },
    });

    await flushPromises();

    expect(wrapper.emitted("defaultBranchUnknown")).toHaveLength(1);
    expect(wrapper.text()).toContain("Choose a default branch");
    wrapper.unmount();
  });
});
