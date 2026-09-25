// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { reactive } from "vue";
import type { GitStatus } from "../domain/git";
import type { Checkout, Repo } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";

const mocks = vi.hoisted(() => ({ listCheckoutFiles: vi.fn(), searchCheckoutFiles: vi.fn() }));

vi.mock("../lib/ipc", () => ({
  listCheckoutFiles: mocks.listCheckoutFiles,
  searchCheckoutFiles: mocks.searchCheckoutFiles,
}));

import InspectorPane from "./InspectorPane.vue";

function checkout(id: string, isMissing = false): Checkout {
  return {
    id,
    repoId: `repo:${id}`,
    path: `/${id}`,
    canonicalPath: `/${id}`,
    isPrimary: true,
    changedFiles: 0,
    isMissing,
    sessions: [],
  };
}

function gitSnapshot(checkoutId: string, status: GitStatus | null = null): ActiveGitSnapshot {
  return reactive({
    checkoutId,
    status,
    viewedPaths: [],
    loading: false,
    statusState: status ? ("ready" as const) : ("error" as const),
    statusError: "",
    changesStatusError: "",
    viewedError: "",
    watchError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
    markViewed: async () => undefined,
  });
}

const repo: Repo = {
  id: "repo:git",
  kind: "git",
  name: "git",
  root: "/repo",
  defaultBranch: "main",
  checkouts: [],
  createdAt: "now",
  lastOpenedAt: "now",
};

function mountInspector(props: {
  checkout: Checkout;
  repo?: Repo;
  gitSnapshot?: ActiveGitSnapshot;
  commandRequest?: { action: "open-file" | "open-changes"; token: number } | null;
}) {
  return mount(InspectorPane, {
    props: { ...props, gitSnapshot: props.gitSnapshot ?? gitSnapshot(props.checkout.id) },
  });
}

describe("InspectorPane", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.searchCheckoutFiles.mockResolvedValue({ entries: [], truncated: false });
  });

  it("opens a selected file in the central document and preserves file selection per checkout", async () => {
    mocks.listCheckoutFiles.mockImplementation(async (checkoutId: string) => ({
      entries: [{ name: `${checkoutId}.txt`, path: `${checkoutId}.txt`, kind: "file" }],
      truncated: false,
    }));
    const wrapper = mountInspector({ checkout: checkout("checkout:first") });
    await flushPromises();

    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    expect(wrapper.emitted("openFile")).toEqual([[{ checkoutId: "checkout:first", path: "checkout:first.txt" }]]);
    expect(wrapper.text()).not.toContain("Selected file");
    expect(wrapper.text()).not.toContain("Preview");

    await wrapper.setProps({ checkout: checkout("checkout:second") });
    await flushPromises();
    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    expect(wrapper.emitted("openFile")?.at(-1)).toEqual([
      { checkoutId: "checkout:second", path: "checkout:second.txt" },
    ]);

    await wrapper.setProps({ checkout: checkout("checkout:first") });
    await flushPromises();
    expect(wrapper.get('[aria-label="Checkout files"] button').classes()).toContain("bg-white/8");
    wrapper.unmount();
  });

  it("shows empty, loading, permission, and missing file-tree states", async () => {
    mocks.listCheckoutFiles.mockResolvedValueOnce({ entries: [], truncated: false });
    const wrapper = mountInspector({ checkout: checkout("empty") });
    expect(wrapper.get('[role="status"]').text()).toContain("Loading files");
    await flushPromises();
    expect(wrapper.text()).toContain("This checkout is empty.");

    mocks.listCheckoutFiles.mockRejectedValueOnce({ code: "permission_denied", message: "denied" });
    await wrapper.setProps({ checkout: checkout("denied") });
    await flushPromises();
    expect(wrapper.text()).toContain("Permission denied");

    await wrapper.setProps({ checkout: checkout("gone", true) });
    expect(wrapper.text()).toContain("Checkout is missing.");
    wrapper.unmount();
  });

  it("virtualizes large trees and fuzzy-searches the checkout file index", async () => {
    const entries = Array.from({ length: 500 }, (_, index) => ({
      name: `file-${index}.txt`,
      path: `file-${index}.txt`,
      kind: "file" as const,
    }));
    mocks.listCheckoutFiles.mockResolvedValue({ entries, truncated: false });
    mocks.searchCheckoutFiles.mockResolvedValue({
      entries: [
        { name: "UserConfig.ts", path: "src/UserConfig.ts", kind: "file" },
        { name: "unrelated.txt", path: "docs/unrelated.txt", kind: "file" },
      ],
      truncated: false,
    });
    const wrapper = mountInspector({ checkout: checkout("large") });
    await flushPromises();

    const fileTree = wrapper.get('[aria-label="Checkout files"]');
    expect(fileTree.findAll("button").length).toBeLessThan(100);
    (fileTree.element as HTMLElement).scrollTop = 500 * 32;
    await fileTree.trigger("scroll");
    expect(fileTree.text()).toContain("file-499.txt");
    expect(fileTree.text()).not.toContain("file-0.txt");

    const search = wrapper.get('input[aria-label="Search files"]');
    await search.setValue("usrcfg");
    await flushPromises();
    expect(mocks.searchCheckoutFiles).toHaveBeenCalledWith("large");
    expect(wrapper.text()).toContain("UserConfig.ts");
    expect(wrapper.text()).not.toContain("unrelated.txt");
    wrapper.unmount();
  });

  it("keeps only Files and Changes navigation and sends a change selection to the main document", async () => {
    mocks.listCheckoutFiles.mockResolvedValue({
      entries: [{ name: "main.ts", path: "src/main.ts", kind: "file" }],
      truncated: false,
    });
    const status: GitStatus = {
      branch: "feature",
      defaultBranch: "main",
      aheadCount: 1,
      files: [{ path: "src/main.ts", status: "M" }],
    };
    const wrapper = mountInspector({ checkout: checkout("git"), repo, gitSnapshot: gitSnapshot("git", status) });
    await flushPromises();
    const sections = wrapper.get('[aria-label="Inspector sections"]').findAll("button");
    expect(sections.map((button) => button.text())).toEqual(["Files", "Changes"]);

    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await wrapper.get('[aria-label="Inspector sections"]').findAll("button")[1].trigger("click");
    await wrapper.get('[aria-label="Changed files"] button').trigger("click");
    expect(wrapper.emitted("openChange")).toEqual([[{ checkoutId: "git", path: "src/main.ts" }]]);

    await wrapper.get('[aria-label="Inspector sections"]').findAll("button")[0].trigger("click");
    expect(wrapper.get('[aria-label="Checkout files"] button').classes()).toContain("bg-white/8");
    wrapper.unmount();
  });

  it("refreshes the file tree when the shared owner reports a Git status event", async () => {
    mocks.listCheckoutFiles
      .mockResolvedValueOnce({ entries: [{ name: "before.txt", path: "before.txt", kind: "file" }], truncated: false })
      .mockResolvedValueOnce({
        entries: [
          { name: "before.txt", path: "before.txt", kind: "file" },
          { name: "after.txt", path: "after.txt", kind: "file" },
        ],
        truncated: false,
      });
    const snapshot = gitSnapshot("git");
    const wrapper = mountInspector({ checkout: checkout("git"), repo, gitSnapshot: snapshot });
    await flushPromises();
    expect(wrapper.text()).not.toContain("after.txt");

    snapshot.statusEventCheckoutId = "git";
    snapshot.statusEventRevision += 1;
    await flushPromises();
    expect(wrapper.text()).toContain("after.txt");
    wrapper.unmount();
  });
});
