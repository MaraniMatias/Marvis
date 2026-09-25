// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, reactive, ref } from "vue";
import type { GitStatus } from "../domain/git";
import type { MainDocument } from "../domain/main-document";
import type { Checkout, Repo } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import DocumentPane from "./DocumentPane.vue";
import FileDiff from "./FileDiff.vue";
import InspectorPane from "./InspectorPane.vue";

const mocks = vi.hoisted(() => ({
  listCheckoutFiles: vi.fn(),
  searchCheckoutFiles: vi.fn(),
  readCheckoutFile: vi.fn(),
  readCheckoutMarkdownImage: vi.fn(),
  getGitDiff: vi.fn(),
  getGitDiffPage: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  listCheckoutFiles: mocks.listCheckoutFiles,
  searchCheckoutFiles: mocks.searchCheckoutFiles,
  readCheckoutFile: mocks.readCheckoutFile,
  readCheckoutMarkdownImage: mocks.readCheckoutMarkdownImage,
  getGitDiff: mocks.getGitDiff,
  getGitDiffPage: mocks.getGitDiffPage,
}));

vi.mock("@git-diff-view/vue", () => ({
  DiffFile: class {
    initTheme() {}
    init() {}
    buildUnifiedDiffLines() {}
  },
  DiffModeEnum: { Unified: 4 },
  DiffView: { template: "<div>Rendered diff</div>" },
}));

function checkout(id: string): Checkout {
  return {
    id,
    repoId: "repo:repo",
    path: `/${id}`,
    canonicalPath: `/${id}`,
    isPrimary: true,
    changedFiles: 0,
    isMissing: false,
    sessions: [],
  };
}

function snapshot(checkoutId: string, files: GitStatus["files"] = []): ActiveGitSnapshot {
  return reactive<ActiveGitSnapshot>({
    checkoutId,
    status: { branch: "feature", defaultBranch: "main", aheadCount: 1, files },
    viewedPaths: [],
    loading: false,
    statusState: "ready",
    statusError: "",
    changesStatusError: "",
    viewedError: "",
    watchError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
    markViewed: vi.fn(async () => undefined),
  });
}

const repo: Repo = {
  id: "repo:repo",
  kind: "git",
  name: "repo",
  root: "/repo",
  defaultBranch: "main",
  checkouts: [],
  createdAt: "now",
  lastOpenedAt: "now",
};

function documentPaneProps(
  document: MainDocument,
  currentCheckout = checkout(document.checkoutId),
  gitSnapshot = snapshot(document.checkoutId),
) {
  return { checkout: currentCheckout, document, gitSnapshot, active: true };
}

describe("DocumentPane", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.listCheckoutFiles.mockResolvedValue({ entries: [], truncated: false });
    mocks.searchCheckoutFiles.mockResolvedValue({ entries: [], truncated: false });
    mocks.readCheckoutMarkdownImage.mockResolvedValue({
      mimeType: "image/png",
      dataBase64: "iVBORw0KGgo=",
      sizeBytes: 8,
    });
    mocks.getGitDiffPage.mockResolvedValue({ path: "", startLine: 0, totalLines: 0, lines: [] });
    mocks.getGitDiff.mockResolvedValue({
      path: "src/app.ts",
      patch: "diff --git a/src/app.ts b/src/app.ts\n@@ -1 +1 @@\n-old\n+new\n",
      isBinary: false,
      large: false,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -1 +1 @@" }],
    });
  });

  it("opens a file selection in the central document, rendering Markdown by default and allowing Code mode", async () => {
    const selected = ref<MainDocument | null>(null);
    const currentCheckout = checkout("checkout:one");
    const gitSnapshot = snapshot(currentCheckout.id, [{ path: "src/app.ts", status: "M" }]);
    mocks.listCheckoutFiles.mockResolvedValue({
      entries: [
        { name: "readme.md", path: "docs/readme.md", kind: "file" },
        { name: "app.ts", path: "src/app.ts", kind: "file" },
      ],
      truncated: false,
    });
    mocks.readCheckoutFile.mockImplementation(async (_checkoutId: string, path: string) => ({
      path,
      content: path.endsWith(".md") ? "# Marvis\n\n[unsafe](javascript:alert(1))" : "const answer: number = 42;",
    }));
    const harness = defineComponent({
      components: { InspectorPane, DocumentPane },
      setup() {
        function openFile(file: { checkoutId: string; path: string }) {
          selected.value = { ...file, source: "file", mode: "view" };
        }
        function openChange(file: { checkoutId: string; path: string }) {
          selected.value = { ...file, source: "change", mode: "diff" };
        }
        return { selected, currentCheckout, gitSnapshot, openFile, openChange };
      },
      data: () => ({ repo }),
      template: `<div>
        <InspectorPane :checkout="currentCheckout" :repo="repo" :git-snapshot="gitSnapshot" @open-file="openFile" @open-change="openChange" />
        <DocumentPane v-if="selected" :checkout="currentCheckout" :document="selected" :git-snapshot="gitSnapshot" @update-mode="selected.mode = $event" />
      </div>`,
    });
    const wrapper = mount(harness);
    await flushPromises();
    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await vi.waitFor(() => expect(wrapper.find('[aria-label="File contents"] article').exists()).toBe(true));
    expect(wrapper.get('[aria-label="File contents"] article').text()).toContain("Marvis");
    expect(wrapper.get('[aria-label="File contents"]').element.querySelector('a[href^="javascript:"]')).toBeNull();
    expect(mocks.readCheckoutFile).toHaveBeenCalledWith(currentCheckout.id, "docs/readme.md");

    await wrapper
      .get('[aria-label="Document mode"]')
      .findAll("button")
      .find((button) => button.text() === "Code")!
      .trigger("click");
    await flushPromises();
    expect(wrapper.get('[aria-label="Source code"]').text()).toContain("# Marvis");

    await wrapper.get('[aria-label="Inspector sections"]').findAll("button")[1].trigger("click");
    expect(wrapper.text()).toContain("docs/readme.md");
    await wrapper.get('[aria-label="Changed files"] button').trigger("click");
    await flushPromises();
    expect(mocks.getGitDiff).toHaveBeenCalledWith(currentCheckout.id, "src/app.ts");
    expect(wrapper.get('[aria-label="Document mode"]').text()).toContain("Diff");
    expect(wrapper.text()).toContain("Rendered diff");
    expect(wrapper.findComponent({ name: "FileDiff" }).props("active")).toBe(true);
    await vi.waitFor(() => expect(gitSnapshot.markViewed).toHaveBeenCalledWith(currentCheckout.id, "src/app.ts"));
    wrapper.unmount();
  });

  it("navigates safe relative Markdown document links within the active checkout", async () => {
    const checkoutId = "checkout:markdown-links";
    const document: MainDocument = { checkoutId, path: "docs/start.md", source: "file", mode: "view" };
    mocks.readCheckoutFile.mockResolvedValue({ path: document.path, content: "[Next guide](../guide.md)" });
    const wrapper = mount(DocumentPane, { props: documentPaneProps(document, checkout(checkoutId)) });
    await vi.waitFor(() => expect(wrapper.find('[aria-label="File contents"] article').exists()).toBe(true));
    await wrapper.get('[aria-label="File contents"] a[href="../guide.md"]').trigger("click");
    expect(wrapper.emitted("openMarkdownLink")).toEqual([["guide.md"]]);
    wrapper.unmount();
  });

  it("offers Diff/View/Code for a Markdown change and only reads it after leaving Diff", async () => {
    const checkoutId = "checkout:markdown-change";
    const document: MainDocument = {
      checkoutId,
      path: "docs/readme.md",
      source: "change",
      mode: "diff",
    };
    const gitSnapshot = snapshot(checkoutId, [{ path: document.path, status: "M" }]);
    mocks.readCheckoutFile.mockResolvedValue({ path: document.path, content: "# Current file" });
    const wrapper = mount(DocumentPane, { props: documentPaneProps(document, checkout(checkoutId), gitSnapshot) });
    await flushPromises();

    expect(
      wrapper
        .get('[aria-label="Document mode"]')
        .findAll("button")
        .map((button) => button.text()),
    ).toEqual(["↗ Zed", "Diff", "View", "Code"]);
    expect(mocks.readCheckoutFile).not.toHaveBeenCalled();

    await wrapper
      .get('[aria-label="Document mode"]')
      .findAll("button")
      .find((button) => button.text() === "View")!
      .trigger("click");
    expect(wrapper.emitted("updateMode")?.at(-1)).toEqual(["view"]);
    await wrapper.setProps({ document: { ...document, mode: "view" } });
    await flushPromises();
    expect(mocks.readCheckoutFile).toHaveBeenCalledWith(checkoutId, document.path);
    await vi.waitFor(() =>
      expect(wrapper.get('[aria-label="File contents"] article').text()).toContain("Current file"),
    );
    wrapper.unmount();
  });

  it("ignores an older file read after switching checkout and document", async () => {
    let resolveOld!: (value: { path: string; content: string }) => void;
    mocks.readCheckoutFile.mockImplementation((_checkoutId: string, path: string) => {
      if (path === "old.ts") return new Promise((resolve) => (resolveOld = resolve));
      return Promise.resolve({ path, content: "new checkout content" });
    });
    const oldDocument: MainDocument = { checkoutId: "checkout:old", path: "old.ts", source: "file", mode: "code" };
    const wrapper = mount(DocumentPane, { props: documentPaneProps(oldDocument) });
    await wrapper.setProps({
      checkout: checkout("checkout:new"),
      document: { checkoutId: "checkout:new", path: "new.ts", source: "file", mode: "code" },
      gitSnapshot: snapshot("checkout:new"),
    });
    await flushPromises();
    resolveOld({ path: "old.ts", content: "stale old checkout content" });
    await flushPromises();
    expect(wrapper.text()).toContain("new checkout content");
    expect(wrapper.text()).not.toContain("stale old checkout content");
    wrapper.unmount();
  });

  it("refreshes a changed file while keeping its reading position", async () => {
    mocks.readCheckoutFile
      .mockResolvedValueOnce({ path: "src/app.ts", content: "const first = true;" })
      .mockResolvedValueOnce({ path: "src/app.ts", content: "const second = true;" });
    const document: MainDocument = { checkoutId: "checkout:refresh", path: "src/app.ts", source: "file", mode: "code" };
    const wrapper = mount(DocumentPane, {
      props: { ...documentPaneProps(document), refreshRevision: 0 },
    });
    await flushPromises();
    const viewport = wrapper.get('[aria-label="File contents"]');
    (viewport.element as HTMLElement).scrollTop = 120;
    (viewport.element as HTMLElement).scrollLeft = 40;
    await viewport.trigger("scroll");

    await wrapper.setProps({ refreshRevision: 1 });
    await flushPromises();
    expect(mocks.readCheckoutFile).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("const second = true;");
    expect((wrapper.get('[aria-label="File contents"]').element as HTMLElement).scrollTop).toBe(120);
    expect((wrapper.get('[aria-label="File contents"]').element as HTMLElement).scrollLeft).toBe(40);
    wrapper.unmount();
  });

  it("does not read the current file for a deleted change", async () => {
    const gitSnapshot = snapshot("checkout:deleted", [{ path: "gone.ts", status: "D" }]);
    const document: MainDocument = { checkoutId: "checkout:deleted", path: "gone.ts", source: "change", mode: "code" };
    const wrapper = mount(DocumentPane, {
      props: documentPaneProps(document, checkout(document.checkoutId), gitSnapshot),
    });
    await flushPromises();
    expect(mocks.readCheckoutFile).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("This file was deleted");
    wrapper.unmount();
  });

  it("keeps the current file readable after its change is removed from Git status", async () => {
    const checkoutId = "checkout:cleaned";
    const gitSnapshot = snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]);
    const document: MainDocument = { checkoutId, path: "src/app.ts", source: "change", mode: "code" };
    mocks.readCheckoutFile.mockResolvedValue({ path: document.path, content: "const stillHere = true;" });
    const wrapper = mount(DocumentPane, { props: documentPaneProps(document, checkout(checkoutId), gitSnapshot) });
    await flushPromises();
    expect(wrapper.text()).toContain("const stillHere = true;");

    gitSnapshot.status!.files = [];
    gitSnapshot.statusRevision += 1;
    await flushPromises();
    expect(wrapper.text()).toContain("const stillHere = true;");
    expect(wrapper.text()).not.toContain("current file is unavailable");
    wrapper.unmount();
  });

  it("pages a large diff and marks it viewed only once its first page is displayed", async () => {
    const checkoutId = "checkout:large-diff";
    const gitSnapshot = snapshot(checkoutId, [{ path: "large.txt", status: "M" }]);
    mocks.getGitDiff.mockResolvedValue({
      path: "large.txt",
      patch: "",
      isBinary: false,
      large: true,
      tooLarge: false,
      totalLines: 2,
      hunks: [{ startLine: 0, endLine: 2, title: "@@ -1 +1 @@" }],
    });
    mocks.getGitDiffPage.mockResolvedValue({
      path: "large.txt",
      startLine: 0,
      totalLines: 2,
      lines: [
        { index: 0, kind: "hunk", text: "@@ -1 +1 @@", oldLineNumber: null, newLineNumber: null },
        { index: 1, kind: "added", text: "+new line", oldLineNumber: null, newLineNumber: 1 },
      ],
    });
    const wrapper = mount(FileDiff, {
      props: { checkout: checkout(checkoutId), gitSnapshot, path: "large.txt", active: false },
    });
    await vi.waitFor(() => expect(wrapper.text()).toContain("+new line"));
    expect(mocks.getGitDiffPage).toHaveBeenCalledWith(checkoutId, "large.txt", 0, 32);
    expect(gitSnapshot.markViewed).not.toHaveBeenCalled();

    await wrapper.setProps({ active: true });
    await vi.waitFor(() => expect(gitSnapshot.markViewed).toHaveBeenCalledWith(checkoutId, "large.txt"));
    wrapper.unmount();
  });

  it("retains the selected diff on status refresh and handles binary and over-limit changes", async () => {
    const checkoutId = "checkout:diff-status";
    const gitSnapshot = snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]);
    const wrapper = mount(FileDiff, {
      props: { checkout: checkout(checkoutId), gitSnapshot, path: "src/app.ts", active: true },
    });
    await flushPromises();
    expect(mocks.getGitDiff).toHaveBeenCalledTimes(1);
    gitSnapshot.statusRevision += 1;
    await flushPromises();
    expect(mocks.getGitDiff).toHaveBeenCalledTimes(2);
    wrapper.unmount();

    for (const result of [
      { isBinary: true, tooLarge: false, message: "Binary file; text diff is unavailable." },
      { isBinary: false, tooLarge: true, message: "This diff exceeds safe preview limits" },
    ]) {
      const isolatedSnapshot = snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]);
      mocks.getGitDiff.mockResolvedValueOnce({
        path: "src/app.ts",
        patch: "",
        isBinary: result.isBinary,
        large: false,
        tooLarge: result.tooLarge,
        totalLines: 0,
        hunks: [],
      });
      const special = mount(FileDiff, {
        props: { checkout: checkout(checkoutId), gitSnapshot: isolatedSnapshot, path: "src/app.ts", active: true },
      });
      await vi.waitFor(() => expect(special.text()).toContain(result.message));
      expect(isolatedSnapshot.markViewed).not.toHaveBeenCalled();
      special.unmount();
    }
  });
});
