// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, reactive, ref } from "vue";
import type { VNodeChild } from "vue";
import type { GitStatus } from "../domain/git";
import type { MainDocument } from "../domain/main-document";
import type { ReviewNote } from "../domain/review";
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
  getGitCheckoutDiffStats: vi.fn(async () => ({})),
  getGitDiffStats: vi.fn(async () => []),
}));

vi.mock("../lib/ipc", () => ({
  listCheckoutFiles: mocks.listCheckoutFiles,
  searchCheckoutFiles: mocks.searchCheckoutFiles,
  readCheckoutFile: mocks.readCheckoutFile,
  readCheckoutMarkdownImage: mocks.readCheckoutMarkdownImage,
  getGitDiff: mocks.getGitDiff,
  getGitDiffPage: mocks.getGitDiffPage,
  // The mounted InspectorPane reads the diff counts; it only renders them when present.
  getGitCheckoutDiffStats: mocks.getGitCheckoutDiffStats,
  getGitDiffStats: mocks.getGitDiffStats,
}));

vi.mock("@git-diff-view/vue", async () => {
  const { defineComponent: component, h: createElement } = await import("vue");
  return {
    DiffFile: class {
      initTheme() {}
      init() {}
      buildUnifiedDiffLines() {}
    },
    DiffModeEnum: { Unified: 4 },
    // Renders the review slots the way the real diff view does for the active line.
    DiffView: component({
      name: "DiffView",
      props: {
        diffViewAddWidget: { type: Boolean, default: false },
        extendData: { type: Object, default: () => ({}) },
      },
      setup:
        (
          props: {
            diffViewAddWidget: boolean;
            extendData: { newFile?: Record<string, { data: ReviewNote[] }> };
          },
          { slots }: { slots: Record<string, ((payload: never) => VNodeChild) | undefined> },
        ) =>
        () =>
          createElement("div", { "data-testid": "diff-view" }, [
            String(props.diffViewAddWidget),
            Object.keys(props.extendData?.newFile ?? {}).join(","),
            slots.extend?.({ data: props.extendData?.newFile?.["1"]?.data ?? [] } as never),
            slots.widget?.({ lineNumber: 1, side: 2, onClose: () => undefined } as never),
          ]),
    }),
  };
});

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

function reviewNote(overrides: Partial<ReviewNote> = {}): ReviewNote {
  return {
    id: "note:1",
    checkoutId: "checkout:repo",
    path: "src/app.ts",
    side: "new",
    lineStart: 1,
    lineEnd: null,
    content: "revisit this calculation",
    code: "+new",
    codeHash: "0000000000000001",
    outdated: false,
    roundId: null,
    status: "draft",
    createdAt: "1",
    updatedAt: "1",
    ...overrides,
  };
}

function reviewApi() {
  return reactive({
    notes: [] as ReviewNote[],
    addNote: vi.fn(async () => true),
    updateNote: vi.fn(async () => true),
    deleteNote: vi.fn(async () => true),
    verifyAnchors: vi.fn(async () => true),
    clearOutdated: vi.fn(async () => true),
    resolveNote: vi.fn(async () => true),
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
  return { checkout: currentCheckout, document, gitSnapshot, review: reviewApi(), active: true };
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
        return { selected, currentCheckout, gitSnapshot, review: reviewApi(), openFile, openChange };
      },
      data: () => ({ repo }),
      template: `<div>
        <InspectorPane :checkout="currentCheckout" :repo="repo" :git-snapshot="gitSnapshot" @open-file="openFile" @open-change="openChange" />
        <DocumentPane v-if="selected" :checkout="currentCheckout" :document="selected" :git-snapshot="gitSnapshot" :review="review" @update-mode="selected.mode = $event" />
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
    await vi.waitFor(() => expect(wrapper.find('[aria-label="Source code"]').exists()).toBe(true));
    expect(wrapper.get('[aria-label="Source code"]').text()).toContain("# Marvis");

    await wrapper.get('[aria-label="Inspector sections"]').findAll("button")[1].trigger("click");
    expect(wrapper.text()).toContain("docs/readme.md");
    await wrapper.get('[aria-label="Changed files"] button').trigger("click");
    await flushPromises();
    expect(mocks.getGitDiff).toHaveBeenCalledWith(currentCheckout.id, "src/app.ts");
    expect(wrapper.get('[aria-label="Document mode"]').text()).toContain("Diff");
    expect(wrapper.find('[data-testid="diff-view"]').exists()).toBe(true);
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

  it("highlights multiline source with Shiki and displays line numbers", async () => {
    const document: MainDocument = {
      checkoutId: "checkout:multiline-source",
      path: "src/example.ts",
      source: "file",
      mode: "code",
    };
    mocks.readCheckoutFile.mockResolvedValue({
      path: document.path,
      content: "const answer: number = 42;\nreturn answer;",
    });
    const wrapper = mount(DocumentPane, { props: documentPaneProps(document) });
    await vi.waitFor(() => expect(wrapper.findAll(".shiki .line")).toHaveLength(2));
    const source = wrapper.get('[aria-label="Source code"]');
    expect(source.findAll(".shiki .line").map((line) => line.text())).toEqual([
      "const answer: number = 42;",
      "return answer;",
    ]);
    expect(source.findAll(".shiki .line > span").length).toBeGreaterThan(0);
    expect(source.findAll(".source-line-number").map((number) => number.text())).toEqual(["1", "2"]);
    wrapper.unmount();
  });

  it("uses compact source rendering above the line bound", async () => {
    const document: MainDocument = {
      checkoutId: "checkout:many-lines",
      path: "logs/output.txt",
      source: "file",
      mode: "code",
    };
    mocks.readCheckoutFile.mockResolvedValue({ path: document.path, content: Array(5001).fill("line").join("\n") });
    const wrapper = mount(DocumentPane, { props: documentPaneProps(document) });
    await flushPromises();
    const source = wrapper.get('[aria-label="Source code"]');
    expect(source.findAll(".source-line-number")).toHaveLength(1);
    expect(source.get(".source-line-number").text().split("\n")).toHaveLength(5001);
    expect(wrapper.get('button[title^="Wrapping is unavailable"]').attributes("disabled")).toBeDefined();
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

  it("keeps the previous file visible while a fast file-to-change selection settles", async () => {
    const checkoutId = "checkout:quick-selection";
    const fileDocument: MainDocument = { checkoutId, path: "a.ts", source: "file", mode: "code" };
    const changeDocument: MainDocument = { checkoutId, path: "b.ts", source: "change", mode: "diff" };
    mocks.readCheckoutFile.mockResolvedValue({ path: "a.ts", content: "const fileA = true;" });
    let resolveDiff!: (value: {
      path: string;
      patch: string;
      isBinary: boolean;
      large: boolean;
      tooLarge: boolean;
      totalLines: number;
      hunks: { startLine: number; endLine: number; title: string }[];
    }) => void;
    mocks.getGitDiff.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveDiff = resolve;
        }),
    );
    const wrapper = mount(DocumentPane, { props: documentPaneProps(fileDocument, checkout(checkoutId)) });
    await flushPromises();
    expect(wrapper.text()).toContain("const fileA = true;");

    await wrapper.setProps({ document: changeDocument });
    await vi.waitFor(() => expect(resolveDiff).toBeTypeOf("function"));
    expect(wrapper.get('[aria-label="File contents"]').text()).toContain("const fileA = true;");
    expect(wrapper.text()).not.toContain("No text hunks are available for this change.");

    resolveDiff({
      path: "b.ts",
      patch: "diff --git a/b.ts b/b.ts\n@@ -1 +1 @@\n-old\n+new\n",
      isBinary: false,
      large: false,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -1 +1 @@" }],
    });
    await flushPromises();
    expect(wrapper.find('[data-testid="diff-view"]').exists()).toBe(true);
    expect(wrapper.text()).not.toContain("const fileA = true;");
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

  it("restores a diff position independently and reports later scrolling", async () => {
    const checkoutId = "checkout:diff-position";
    const document: MainDocument = { checkoutId, path: "src/app.ts", source: "change", mode: "diff" };
    const wrapper = mount(DocumentPane, {
      props: { ...documentPaneProps(document), diffScrollTop: 72 },
    });
    await flushPromises();
    const viewport = wrapper.get('[aria-label="Diff contents"]');
    expect((viewport.element as HTMLElement).scrollTop).toBe(72);
    (viewport.element as HTMLElement).scrollTop = 144;
    await viewport.trigger("scroll");
    expect(wrapper.emitted("diffPositionChanged")).toEqual([[144]]);
    wrapper.unmount();
  });

  it("restores Markdown reading position after relative images finish decoding", async () => {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, "decode");
    const decode = vi.fn(function (this: HTMLImageElement) {
      const viewport = this.closest(".document-pane")?.querySelector('[aria-label="File contents"]') as HTMLElement;
      viewport.scrollTop = 0;
      return Promise.resolve();
    });
    Object.defineProperty(HTMLImageElement.prototype, "decode", { configurable: true, value: decode });
    const document: MainDocument = {
      checkoutId: "checkout:markdown-scroll",
      path: "docs/readme.md",
      source: "file",
      mode: "view",
    };
    mocks.readCheckoutFile.mockResolvedValue({ path: document.path, content: "![preview](image.png)" });
    const wrapper = mount(DocumentPane, {
      props: {
        ...documentPaneProps(document),
        readingPosition: { top: 240, left: 12 },
      },
    });
    try {
      await vi.waitFor(() => expect(decode).toHaveBeenCalledOnce());
      expect((wrapper.get('[aria-label="File contents"]').element as HTMLElement).scrollTop).toBe(240);
      expect((wrapper.get('[aria-label="File contents"]').element as HTMLElement).scrollLeft).toBe(12);
    } finally {
      wrapper.unmount();
      if (descriptor) Object.defineProperty(HTMLImageElement.prototype, "decode", descriptor);
      else delete (HTMLImageElement.prototype as { decode?: () => Promise<void> }).decode;
    }
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
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot,
        review: reviewApi(),
        path: "large.txt",
        active: false,
        scrollTop: 0,
      },
    });
    await vi.waitFor(() => expect(wrapper.text()).toContain("+new line"));
    expect(mocks.getGitDiffPage).toHaveBeenCalledWith(checkoutId, "large.txt", 0, 32);
    expect(gitSnapshot.markViewed).not.toHaveBeenCalled();

    await wrapper.setProps({ active: true });
    await vi.waitFor(() => expect(gitSnapshot.markViewed).toHaveBeenCalledWith(checkoutId, "large.txt"));
    wrapper.unmount();
  });

  it("feeds saved notes to the hunk renderer so they appear under their line", async () => {
    const checkoutId = "checkout:hunk-notes";
    const gitSnapshot = snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]);
    const note = reviewNote({ id: "note:1", lineStart: 1 });
    const review = reviewApi();
    review.notes = [note, reviewNote({ id: "note:2", path: "src/other.ts", lineStart: 5 })];
    const wrapper = mount(FileDiff, {
      props: { checkout: checkout(checkoutId), gitSnapshot, review, path: "src/app.ts", active: true, scrollTop: 0 },
    });
    await flushPromises();

    const view = wrapper.get('[data-testid="diff-view"]');
    expect(view.text()).toContain("true");
    expect(view.text()).toContain("1");
    expect(wrapper.find('[data-testid="diff-view"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("saves a note from the hunk widget with the code of the commented line", async () => {
    const checkoutId = "checkout:hunk-widget";
    const gitSnapshot = snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]);
    const review = reviewApi();
    const wrapper = mount(FileDiff, {
      props: { checkout: checkout(checkoutId), gitSnapshot, review, path: "src/app.ts", active: true, scrollTop: 0 },
    });
    await flushPromises();

    const form = wrapper.get('form[aria-label="New review note"]');
    await form.get('textarea[aria-label="Review note"]').setValue("guard the user");
    await form.trigger("submit");
    await flushPromises();

    expect(review.addNote).toHaveBeenCalledWith({
      path: "src/app.ts",
      side: "new",
      lineStart: 1,
      content: "guard the user",
      code: "new",
    });
    wrapper.unmount();
  });

  it("adds an inline review note to a virtualized diff row with the captured code", async () => {
    const checkoutId = "checkout:large-note";
    const gitSnapshot = snapshot(checkoutId, [{ path: "large.txt", status: "M" }]);
    mocks.getGitDiff.mockResolvedValue({
      path: "large.txt",
      patch: "@@ -0,0 +1,2 @@\n+first line\n+second line\n",
      isBinary: false,
      large: true,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -0,0 +1,2 @@" }],
    });
    mocks.getGitDiffPage.mockResolvedValue({
      path: "large.txt",
      startLine: 0,
      totalLines: 3,
      lines: [
        { index: 0, kind: "hunk", text: "@@ -0,0 +1,2 @@", oldLineNumber: null, newLineNumber: null },
        { index: 1, kind: "added", text: "+first line", oldLineNumber: null, newLineNumber: 1 },
        { index: 2, kind: "added", text: "+second line", oldLineNumber: null, newLineNumber: 2 },
      ],
    });
    const review = reviewApi();
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot,
        review,
        path: "large.txt",
        active: true,
        scrollTop: 0,
      },
    });
    await vi.waitFor(() => expect(wrapper.text()).toContain("+first line"));

    await wrapper.get('[aria-label="Add review note on line 1"]').trigger("click");
    const textarea = wrapper.get('textarea[aria-label="Review note"]');
    await textarea.setValue("revisit this calculation");
    await wrapper.get('form[aria-label="New review note"]').trigger("submit");
    await flushPromises();

    expect(review.addNote).toHaveBeenCalledWith({
      path: "large.txt",
      side: "new",
      lineStart: 1,
      content: "revisit this calculation",
      code: "first line",
    });
    wrapper.unmount();
  });

  it("extends an open draft into a range and captures every line inside it", async () => {
    const checkoutId = "checkout:range-note";
    const gitSnapshot = snapshot(checkoutId, [{ path: "large.txt", status: "M" }]);
    mocks.getGitDiff.mockResolvedValue({
      path: "large.txt",
      patch: "@@ -0,0 +1,3 @@\n+first line\n+second line\n+third line\n",
      isBinary: false,
      large: true,
      tooLarge: false,
      totalLines: 4,
      hunks: [{ startLine: 0, endLine: 4, title: "@@ -0,0 +1,3 @@" }],
    });
    mocks.getGitDiffPage.mockResolvedValue({
      path: "large.txt",
      startLine: 0,
      totalLines: 4,
      lines: [
        { index: 0, kind: "hunk", text: "@@ -0,0 +1,3 @@", oldLineNumber: null, newLineNumber: null },
        { index: 1, kind: "added", text: "+first line", oldLineNumber: null, newLineNumber: 1 },
        { index: 2, kind: "added", text: "+second line", oldLineNumber: null, newLineNumber: 2 },
        { index: 3, kind: "added", text: "+third line", oldLineNumber: null, newLineNumber: 3 },
      ],
    });
    const review = reviewApi();
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot,
        review,
        path: "large.txt",
        active: true,
        scrollTop: 0,
      },
    });
    await vi.waitFor(() => expect(wrapper.text()).toContain("+first line"));

    await wrapper.get('[aria-label="Add review note on line 1"]').trigger("click");
    // A second click on the same side widens the range instead of starting a new note.
    await wrapper.get('[aria-label="Add review note on line 3"]').trigger("click");
    expect(wrapper.get("form[aria-label='New review note']").text()).toContain("new lines 1-3");

    await wrapper.get('textarea[aria-label="Review note"]').setValue("these three lines belong together");
    await wrapper.get('form[aria-label="New review note"]').trigger("submit");
    await flushPromises();

    expect(review.addNote).toHaveBeenCalledWith({
      path: "large.txt",
      side: "new",
      lineStart: 1,
      lineEnd: 3,
      content: "these three lines belong together",
      code: "first line\nsecond line\nthird line",
    });
    wrapper.unmount();
  });

  it("reports the anchor text of visible notes and holds drifted ones back", async () => {
    const checkoutId = "checkout:anchor-verify";
    const gitSnapshot = snapshot(checkoutId, [{ path: "large.txt", status: "M" }]);
    mocks.getGitDiff.mockResolvedValue({
      path: "large.txt",
      patch: "@@ -0,0 +1,2 @@\n+first line\n+second line\n",
      isBinary: false,
      large: true,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -0,0 +1,2 @@" }],
    });
    mocks.getGitDiffPage.mockResolvedValue({
      path: "large.txt",
      startLine: 0,
      totalLines: 3,
      lines: [
        { index: 0, kind: "hunk", text: "@@ -0,0 +1,2 @@", oldLineNumber: null, newLineNumber: null },
        { index: 1, kind: "added", text: "+first line", oldLineNumber: null, newLineNumber: 1 },
        { index: 2, kind: "added", text: "+second line", oldLineNumber: null, newLineNumber: 2 },
      ],
    });
    const review = reviewApi();
    review.notes = [
      reviewNote({ id: "note:1", path: "large.txt", lineStart: 1, code: "first line" }),
      reviewNote({ id: "note:2", path: "large.txt", lineStart: 2, code: "an older line", outdated: true }),
    ];
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot,
        review,
        path: "large.txt",
        active: true,
        scrollTop: 0,
      },
    });
    await vi.waitFor(() =>
      expect(review.verifyAnchors).toHaveBeenCalledWith("large.txt", [
        { id: "note:1", currentCode: "first line" },
        { id: "note:2", currentCode: "second line" },
      ]),
    );

    // A drifted note says so, and accepting it is the only way out.
    expect(wrapper.text()).toContain("outdated");
    await wrapper.get('[aria-label="Accept note on line 2 even though the line changed"]').trigger("click");
    expect(review.clearOutdated).toHaveBeenCalledWith("note:2");
    wrapper.unmount();
  });

  it("judges each note from the diff and only offers resolve when it can back it", async () => {
    const checkoutId = "checkout:verdict";
    const gitSnapshot = snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]);
    // Hunk mode: the whole patch is in memory, so an absent anchor really is a deleted line.
    mocks.getGitDiff.mockResolvedValue({
      path: "src/app.ts",
      patch: "@@ -1,2 +1,2 @@\n const a = 1;\n+const b = 2;\n",
      isBinary: false,
      large: false,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -1,2 +1,2 @@" }],
    });
    const review = reviewApi();
    review.notes = [
      reviewNote({ id: "note:kept", lineStart: 1, status: "sent" }),
      reviewNote({ id: "note:gone", lineStart: 40, status: "sent" }),
    ];
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot,
        review,
        path: "src/app.ts",
        active: true,
        scrollTop: 0,
      },
    });
    await vi.waitFor(() => expect(wrapper.text()).toContain("revisit this calculation"));

    // Unchanged and deleted can both be resolved, and they are reported apart.
    expect(wrapper.text()).toContain("The line this note points at is unchanged.");
    expect(wrapper.text()).toContain("The line this note points at no longer exists.");
    const resolvable = wrapper.findAll('[aria-label^="Mark note on line"]');
    expect(resolvable.map((button) => button.attributes("aria-label"))).toEqual([
      "Mark note on line 1 as resolved",
      "Mark note on line 40 as resolved",
    ]);
    wrapper.unmount();
  });

  it("offers to resolve an attended note and not a drifted one", async () => {
    const checkoutId = "checkout:resolve-note";
    const gitSnapshot = snapshot(checkoutId, [{ path: "large.txt", status: "M" }]);
    mocks.getGitDiff.mockResolvedValue({
      path: "large.txt",
      patch: "@@ -0,0 +1,2 @@\n+first line\n+second line\n",
      isBinary: false,
      large: true,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -0,0 +1,2 @@" }],
    });
    mocks.getGitDiffPage.mockResolvedValue({
      path: "large.txt",
      startLine: 0,
      totalLines: 3,
      lines: [
        { index: 0, kind: "hunk", text: "@@ -0,0 +1,2 @@", oldLineNumber: null, newLineNumber: null },
        { index: 1, kind: "added", text: "+first line", oldLineNumber: null, newLineNumber: 1 },
        { index: 2, kind: "added", text: "+second line", oldLineNumber: null, newLineNumber: 2 },
      ],
    });
    const review = reviewApi();
    review.notes = [
      reviewNote({ id: "note:1", path: "large.txt", lineStart: 1, status: "sent" }),
      // A draft was never delivered, so there is nothing to resolve.
      reviewNote({ id: "note:2", path: "large.txt", lineStart: 2, status: "draft" }),
      reviewNote({ id: "note:3", path: "large.txt", lineStart: 2, status: "sent", outdated: true }),
    ];
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot,
        review,
        path: "large.txt",
        active: true,
        scrollTop: 0,
      },
    });
    await vi.waitFor(() => expect(wrapper.text()).toContain("+first line"));

    // Only the delivered, non-drifted note can be resolved, and only by the user.
    const resolveButtons = wrapper.findAll('[aria-label="Mark note on line 1 as resolved"]');
    expect(resolveButtons).toHaveLength(1);
    await resolveButtons[0].trigger("click");
    expect(review.resolveNote).toHaveBeenCalledWith("note:1");
    wrapper.unmount();
  });

  it("retains the selected diff on status refresh and handles binary and over-limit changes", async () => {
    const checkoutId = "checkout:diff-status";
    const gitSnapshot = snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]);
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot,
        review: reviewApi(),
        path: "src/app.ts",
        active: true,
        scrollTop: 0,
      },
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
        props: {
          checkout: checkout(checkoutId),
          gitSnapshot: isolatedSnapshot,
          review: reviewApi(),
          path: "src/app.ts",
          active: true,
          scrollTop: 0,
        },
      });
      await vi.waitFor(() => expect(special.text()).toContain(result.message));
      expect(isolatedSnapshot.markViewed).not.toHaveBeenCalled();
      special.unmount();
    }
  });
});
