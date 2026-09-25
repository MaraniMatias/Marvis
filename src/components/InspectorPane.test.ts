// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { reactive } from "vue";
import type { GitStatus } from "../domain/git";
import type { Checkout, Repo } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";

const mocks = vi.hoisted(() => ({
  listCheckoutFiles: vi.fn(),
  readCheckoutFile: vi.fn(),
  readCheckoutMarkdownImage: vi.fn(),
  searchCheckoutFiles: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  listCheckoutFiles: mocks.listCheckoutFiles,
  readCheckoutFile: mocks.readCheckoutFile,
  readCheckoutMarkdownImage: mocks.readCheckoutMarkdownImage,
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

function mountInspector(props: {
  checkout: Checkout;
  repo?: Repo;
  gitSnapshot?: ActiveGitSnapshot;
  commandRequest?: { action: "open-file" | "open-changes" | "open-preview"; token: number } | null;
}) {
  return mount(InspectorPane, {
    props: { ...props, gitSnapshot: props.gitSnapshot ?? gitSnapshot(props.checkout.id) },
  });
}

describe("InspectorPane", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.searchCheckoutFiles.mockResolvedValue({ entries: [], truncated: false });
    mocks.readCheckoutMarkdownImage.mockResolvedValue({
      mimeType: "image/png",
      dataBase64: "iVBORw0KGgo=",
      sizeBytes: 8,
    });
  });

  it("clears selection and loads files from the newly active checkout", async () => {
    mocks.listCheckoutFiles.mockImplementation(async (checkoutId: string) => ({
      entries: [{ name: `${checkoutId}.txt`, path: `${checkoutId}.txt`, kind: "file" }],
      truncated: false,
    }));
    mocks.readCheckoutFile.mockImplementation(async (_checkoutId: string, path: string) => ({
      path,
      content: `contents:${path}`,
    }));
    const wrapper = mountInspector({ checkout: checkout("checkout:first") });
    await flushPromises();

    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("contents:checkout:first.txt");

    await wrapper.setProps({ checkout: checkout("checkout:second") });
    await flushPromises();
    expect(wrapper.text()).not.toContain("contents:checkout:first.txt");
    expect(wrapper.text()).toContain("checkout:second.txt");
    expect(wrapper.text()).toContain("Select a file to read it.");
    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await flushPromises();
    expect(mocks.readCheckoutFile).toHaveBeenLastCalledWith("checkout:second", "checkout:second.txt");
    expect(wrapper.text()).toContain("contents:checkout:second.txt");
  });

  it("shows empty, loading, permission, and missing states", async () => {
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

  it("decorates changed files in the tree with Git status", async () => {
    mocks.listCheckoutFiles.mockImplementation(async (_checkoutId: string, path: string) => ({
      entries:
        path === "."
          ? [{ name: "src", path: "src", kind: "directory" }]
          : [{ name: "main.ts", path: "src/main.ts", kind: "file" }],
      truncated: false,
    }));
    const status: GitStatus = {
      branch: "feature",
      defaultBranch: "main",
      aheadCount: 1,
      files: [{ path: "src/main.ts", status: "M" }],
    };
    const repo = {
      id: "repo:git",
      kind: "git" as const,
      name: "git",
      root: "/repo",
      defaultBranch: "main",
      checkouts: [],
      createdAt: "now",
      lastOpenedAt: "now",
    };
    const wrapper = mountInspector({ checkout: checkout("git"), repo, gitSnapshot: gitSnapshot("git", status) });
    await flushPromises();
    expect(wrapper.findAll("button").map((button) => button.text())).toContain("Changes");

    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await flushPromises();
    expect(wrapper.get('[aria-label="Checkout files"]').text()).toContain("M");
    wrapper.unmount();
  });

  it("refreshes the file tree when the shared owner reports a Git status event", async () => {
    mocks.listCheckoutFiles
      .mockResolvedValueOnce({
        entries: [{ name: "before.txt", path: "before.txt", kind: "file" }],
        truncated: false,
      })
      .mockResolvedValueOnce({
        entries: [
          { name: "before.txt", path: "before.txt", kind: "file" },
          { name: "after.txt", path: "after.txt", kind: "file" },
        ],
        truncated: false,
      });
    const repo = {
      id: "repo:git",
      kind: "git" as const,
      name: "git",
      root: "/repo",
      defaultBranch: "main",
      checkouts: [],
      createdAt: "now",
      lastOpenedAt: "now",
    };
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

  it("offers Preview for a selected file and renders safe Markdown with checkout-relative images", async () => {
    mocks.listCheckoutFiles.mockResolvedValue({
      entries: [{ name: "readme.md", path: "docs/readme.md", kind: "file" }],
      truncated: false,
    });
    mocks.readCheckoutFile.mockResolvedValue({
      path: "docs/readme.md",
      content: "# Hello\n\n[unsafe](javascript:alert(1))\n\n![local](../images/photo.png)",
    });
    const wrapper = mountInspector({ checkout: checkout("markdown") });
    await flushPromises();

    expect(wrapper.findAll("button").map((button) => button.text())).not.toContain("Preview");
    expect(wrapper.findAll("button").map((button) => button.text())).not.toContain("Changes");
    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await flushPromises();
    expect(wrapper.findAll("button").map((button) => button.text())).toContain("Preview");
    await vi.waitFor(() => {
      expect(mocks.readCheckoutMarkdownImage).toHaveBeenCalledWith("markdown", "docs/readme.md", "../images/photo.png");
    });

    await wrapper
      .get('[aria-label="Inspector sections"]')
      .findAll("button")
      .find((button) => button.text() === "Preview")!
      .trigger("click");
    await flushPromises();
    const preview = wrapper.get('[aria-label="File preview"] article');
    expect(preview.text()).toContain("Hello");
    expect(preview.element.querySelector("a[href^='javascript:'], script, img[onerror]")).toBeNull();
    expect(preview.element.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,iVBORw0KGgo=");
    wrapper.unmount();
  });

  it("ignores a Markdown preview result that completes after another file is selected", async () => {
    mocks.listCheckoutFiles.mockResolvedValue({
      entries: [
        { name: "old.md", path: "old.md", kind: "file" },
        { name: "new.md", path: "new.md", kind: "file" },
      ],
      truncated: false,
    });
    mocks.readCheckoutFile.mockImplementation(async (_checkoutId: string, path: string) => ({
      path,
      content: path === "old.md" ? "# Old\n\n![old](old.png)" : "# New",
    }));
    let resolveImage!: (image: { mimeType: string; dataBase64: string; sizeBytes: number }) => void;
    mocks.readCheckoutMarkdownImage.mockReturnValue(
      new Promise((resolve) => {
        resolveImage = resolve;
      }),
    );
    const wrapper = mountInspector({ checkout: checkout("stale-preview") });
    await flushPromises();

    const files = wrapper.get('[aria-label="Checkout files"]');
    await files
      .findAll("button")
      .find((button) => button.text().includes("old.md"))!
      .trigger("click");
    await vi.waitFor(() => expect(mocks.readCheckoutMarkdownImage).toHaveBeenCalled());

    await files
      .findAll("button")
      .find((button) => button.text().includes("new.md"))!
      .trigger("click");
    await flushPromises();
    await wrapper
      .get('[aria-label="Inspector sections"]')
      .findAll("button")
      .find((button) => button.text() === "Preview")!
      .trigger("click");
    resolveImage({ mimeType: "image/png", dataBase64: "iVBORw0KGgo=", sizeBytes: 8 });
    await flushPromises();

    const preview = wrapper.get('[aria-label="File preview"] article');
    expect(preview.text()).toContain("New");
    expect(preview.text()).not.toContain("Old");
    expect(preview.element.querySelector("img")).toBeNull();
    wrapper.unmount();
  });

  it("previews non-Markdown source as plain text instead of rendering it as HTML", async () => {
    mocks.listCheckoutFiles.mockResolvedValue({
      entries: [{ name: "example.ts", path: "src/example.ts", kind: "file" }],
      truncated: false,
    });
    mocks.readCheckoutFile.mockResolvedValue({
      path: "src/example.ts",
      content: "const source = '<script>alert(1)</script>';",
    });
    const wrapper = mountInspector({ checkout: checkout("source") });
    await flushPromises();
    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await flushPromises();
    await wrapper
      .get('[aria-label="Inspector sections"]')
      .findAll("button")
      .find((button) => button.text() === "Preview")!
      .trigger("click");

    expect(wrapper.get('[aria-label="File preview"] pre').text()).toContain("<script>");
    expect(wrapper.get('[aria-label="File preview"]').element.querySelector("script")).toBeNull();
    wrapper.unmount();
  });

  it("shows actionable missing, binary, and large-file preview states", async () => {
    mocks.listCheckoutFiles.mockResolvedValue({
      entries: [
        { name: "missing.txt", path: "missing.txt", kind: "file" },
        { name: "binary.bin", path: "binary.bin", kind: "file" },
        { name: "large.txt", path: "large.txt", kind: "file" },
      ],
      truncated: false,
    });
    mocks.readCheckoutFile.mockImplementation(async (_id: string, path: string) => {
      if (path === "missing.txt") throw { code: "folder_missing", message: "gone" };
      if (path === "binary.bin") throw { code: "binary_file", message: "binary" };
      throw { code: "file_too_large", message: "large" };
    });
    const wrapper = mountInspector({ checkout: checkout("file-states") });
    await flushPromises();

    for (const [filename, message] of [
      ["missing.txt", "File or folder no longer exists."],
      ["binary.bin", "This file is binary or is not valid UTF-8."],
      ["large.txt", "This file is larger than the preview size limit."],
    ]) {
      const filesTab = wrapper
        .get('[aria-label="Inspector sections"]')
        .findAll("button")
        .find((button) => button.text() === "Files");
      if (filesTab) await filesTab.trigger("click");
      await wrapper
        .get('[aria-label="Checkout files"]')
        .findAll("button")
        .find((button) => button.text().includes(filename))!
        .trigger("click");
      await flushPromises();
      await wrapper
        .get('[aria-label="Inspector sections"]')
        .findAll("button")
        .find((button) => button.text() === "Preview")!
        .trigger("click");
      expect(wrapper.get('[aria-label="File preview"] [role="alert"]').text()).toBe(message);
    }
    wrapper.unmount();
  });
});
