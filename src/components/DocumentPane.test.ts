// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import type { VueWrapper } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { defineComponent, reactive, ref } from "vue";
import type { VNodeChild } from "vue";
import type { GitStatus } from "../domain/git";
import type { DocumentMode, MainView } from "../domain/main-document";
import type { ReviewNote } from "../domain/review";
import type { Checkout, Repo } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { useToasts } from "../presentation/toasts";
import { isMarkdownPath } from "../presentation/markdown-preview";
import DocumentPane from "./DocumentPane.vue";
import FileDiff from "./FileDiff.vue";
import InspectorPane from "./InspectorPane.vue";
import MainPane from "./MainPane.vue";

const { toasts, dismiss } = useToasts();

const mocks = vi.hoisted(() => ({
  listCheckoutFiles: vi.fn(),
  searchCheckoutFiles: vi.fn(),
  readCheckoutFile: vi.fn(),
  writeCheckoutFile: vi.fn(),
  readCheckoutMarkdownImage: vi.fn(),
  getGitDiff: vi.fn(),
  getGitDiffPage: vi.fn(),
  getGitCheckoutDiffStats: vi.fn(async () => ({})),
  getGitDiffStats: vi.fn(async () => []),
  loadTerminalLayout: vi.fn(async () => null),
  saveTerminalLayout: vi.fn(async () => undefined),
}));

vi.mock("../lib/ipc", () => ({
  listCheckoutFiles: mocks.listCheckoutFiles,
  searchCheckoutFiles: mocks.searchCheckoutFiles,
  readCheckoutFile: mocks.readCheckoutFile,
  writeCheckoutFile: mocks.writeCheckoutFile,
  readCheckoutMarkdownImage: mocks.readCheckoutMarkdownImage,
  getGitDiff: mocks.getGitDiff,
  getGitDiffPage: mocks.getGitDiffPage,
  // The mounted InspectorPane reads the diff counts; it only renders them when present.
  getGitCheckoutDiffStats: mocks.getGitCheckoutDiffStats,
  getGitDiffStats: mocks.getGitDiffStats,
  // MainPane imports the terminal pane; only its stub is mounted.
  loadTerminalLayout: mocks.loadTerminalLayout,
  saveTerminalLayout: mocks.saveTerminalLayout,
}));

vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
}));

vi.mock("reka-ui", async () => {
  const { h: createElement } = await import("vue");
  // The document toolbar's popover always renders here, so a grammar can be picked without
  // driving the open state, the way App.test.ts stubs the titlebar's own popover. Plain options
  // objects rather than defineComponent, which would be one more component in a file that already
  // warns about them.
  const passThrough = (name: string) => ({
    name,
    setup:
      (
        _props: unknown,
        context: { attrs: Record<string, unknown>; slots: Record<string, (() => unknown) | undefined> },
      ) =>
      () =>
        createElement("div", context.attrs, context.slots.default?.() as never),
  });
  return {
    PopoverRoot: passThrough("PopoverRoot"),
    PopoverTrigger: passThrough("PopoverTrigger"),
    // The list is portalled out of the pane so the CodeMirror view underneath cannot paint over
    // it. The portal is the library's and is the one thing here that is worth stubbing as itself:
    // a pass-through stands in for it, and where the content lands is not this file's business.
    PopoverPortal: passThrough("PopoverPortal"),
    PopoverContent: passThrough("PopoverContent"),
  };
});

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
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
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
  path: string | null,
  mode: DocumentMode = "code",
  currentCheckout = checkout("checkout:one"),
  gitSnapshot = snapshot(currentCheckout.id),
) {
  return { checkout: currentCheckout, path, mode, gitSnapshot };
}

/** The grammar rows, by the label each one shows. */
function languageRowLabels(wrapper: VueWrapper): (string | undefined)[] {
  return wrapper
    .get('[role="listbox"]')
    .findAll('[role="option"]')
    .map((row) => row.findAll("span")[0]?.text());
}

async function pickLanguage(wrapper: VueWrapper, label: string) {
  const row = wrapper
    .get('[role="listbox"]')
    .findAll('[role="option"]')
    .find((option) => option.text().startsWith(label));
  if (!row) throw new Error(`no row for ${label}`);
  await row.trigger("click");
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
    for (const toast of [...toasts.value]) dismiss(toast.id);
  });

  it("opens a file selection in the central document, rendering Markdown by default and allowing Code mode", async () => {
    const view = ref<MainView>({ kind: "terminal", sessionId: null });
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
      components: { InspectorPane, MainPane },
      setup() {
        function openFile(file: { checkoutId: string; path: string }) {
          view.value = { kind: "document", path: file.path, mode: "view" };
        }
        function openChange(file: { checkoutId: string; path: string }) {
          view.value = { kind: "diff", path: file.path };
        }
        return { view, currentCheckout, gitSnapshot, review: reviewApi(), openFile, openChange };
      },
      data: () => ({ repo }),
      template: `<div>
        <InspectorPane :checkout="currentCheckout" :repo="repo" :git-snapshot="gitSnapshot" @open-file="openFile" @open-change="openChange" />
        <MainPane
          :checkout="currentCheckout"
          :view="view"
          :ready="true"
          :git-snapshot="gitSnapshot"
          :review="review"
          :active-session-id="null"
          :is-opening="false"
          @update-document-mode="view = { ...view, mode: $event }"
        />
      </div>`,
    });
    const wrapper = mount(harness, {
      global: { stubs: { SessionPane: true } },
    });
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
    // A change is the diff view now, not a mode of the document view (F.1, F.3).
    expect(mocks.getGitDiff).toHaveBeenCalledWith(currentCheckout.id, "src/app.ts");
    expect((wrapper.get("#main-view-document").element as HTMLElement).style.display).toBe("none");
    expect((wrapper.get("#main-view-diff").element as HTMLElement).style.display).not.toBe("none");
    expect(wrapper.find('[data-testid="diff-view"]').exists()).toBe(true);

    view.value = { kind: "diff", path: null };
    await vi.waitFor(() => expect(wrapper.get(".diff-file-header").text()).toContain("src/app.ts"));
    expect(wrapper.get('[aria-label="File diff"] header').text()).toContain("All changes");
    // The file's own diff is a diff of this same component, loaded the same lazy way.
    await wrapper.get(".diff-file-header").trigger("click");
    await vi.waitFor(() => expect(wrapper.findAll('[data-testid="diff-view"]')).toHaveLength(1));
    wrapper.unmount();
  });

  it("navigates safe relative Markdown document links within the active checkout", async () => {
    const checkoutId = "checkout:markdown-links";
    mocks.readCheckoutFile.mockResolvedValue({ path: "docs/start.md", content: "[Next guide](../guide.md)" });
    const wrapper = mount(DocumentPane, {
      props: documentPaneProps("docs/start.md", "view", checkout(checkoutId)),
    });
    await vi.waitFor(() => expect(wrapper.find('[aria-label="File contents"] article').exists()).toBe(true));
    await wrapper.get('[aria-label="File contents"] a[href="../guide.md"]').trigger("click");
    expect(wrapper.emitted("openMarkdownLink")).toEqual([["guide.md"]]);
    wrapper.unmount();
  });

  it("copies the absolute checkout path and reports success", async () => {
    mocks.readCheckoutFile.mockResolvedValue({ path: "src/app.ts", content: "const answer = 42;" });
    const wrapper = mount(DocumentPane, { props: documentPaneProps("src/app.ts") });
    await flushPromises();

    await wrapper.get('[aria-label="Copy file path"]').trigger("click");

    expect(writeText).toHaveBeenCalledWith("/checkout:one/src/app.ts");
    expect(toasts.value.at(-1)?.message).toBe("File path copied.");
    wrapper.unmount();
  });

  it("keeps a Code draft visible and saves it with its original content", async () => {
    mocks.readCheckoutFile.mockResolvedValue({ path: "src/app.ts", content: "const answer = 42;" });
    mocks.writeCheckoutFile.mockResolvedValue(undefined);
    const wrapper = mount(DocumentPane, { props: documentPaneProps("src/app.ts") });
    await vi.waitFor(() => expect(wrapper.find(".cm-content").exists()).toBe(true));

    const editor = wrapper.get(".cm-content").element as HTMLElement;
    editor.textContent = "const answer = 43;";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "3" }));
    await flushPromises();

    expect(wrapper.find('[aria-label="Unsaved changes"]').exists()).toBe(true);
    await wrapper.get('button[aria-label="Save"]').trigger("click");
    await flushPromises();
    expect(mocks.writeCheckoutFile).toHaveBeenCalledWith(
      "checkout:one",
      "src/app.ts",
      "const answer = 43;",
      "const answer = 42;",
    );
    expect(wrapper.find('[aria-label="Unsaved changes"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("offers View/Code only for a Markdown file, and no mode control for anything else", async () => {
    mocks.readCheckoutFile.mockResolvedValue({ path: "docs/readme.md", content: "# Current file" });
    const wrapper = mount(DocumentPane, {
      props: documentPaneProps("docs/readme.md", "view"),
    });
    await flushPromises();

    expect(
      wrapper
        .get('[aria-label="Document mode"]')
        .findAll("button")
        .map((button) => button.text()),
    ).toEqual(["View", "Code"]);

    await wrapper
      .get('[aria-label="Document mode"]')
      .findAll("button")
      .find((button) => button.text() === "Code")!
      .trigger("click");
    expect(wrapper.emitted("updateMode")?.at(-1)).toEqual(["code"]);
    wrapper.unmount();
  });

  it("shows no mode control for a file that has no modes to switch", async () => {
    mocks.readCheckoutFile.mockResolvedValue({ path: "src/app.ts", content: "const answer: number = 42;" });
    const wrapper = mount(DocumentPane, { props: documentPaneProps("src/app.ts") });
    await flushPromises();

    expect(wrapper.find('[aria-label="Document mode"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Wrap");
    expect(wrapper.emitted("updateMode")).toBeUndefined();
    // A grammar can still be forced onto a file that has no modes, which is the whole point.
    expect(wrapper.get('[data-testid="language-trigger"]').text()).toBe("TypeScript");
    wrapper.unmount();
  });

  it("puts the grammar control on the same row, before the mode Markdown has", async () => {
    mocks.readCheckoutFile.mockResolvedValue({ path: "docs/readme.md", content: "# Current file" });
    const wrapper = mount(DocumentPane, {
      props: documentPaneProps("docs/readme.md", "code"),
    });
    await flushPromises();

    const language = wrapper.get('[aria-label="Highlight language"]');
    const mode = wrapper.get('[aria-label="Document mode"]');
    // In Auto the button says what the extension already decided, and it is not a mode: a grammar
    // applies to a non-Markdown file too, so it cannot live inside that group.
    expect(wrapper.get('[data-testid="language-trigger"]').text()).toBe("Markdown");
    expect(language.element.compareDocumentPosition(mode.element) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // The picker is a crumb that opens a list, not a button: the same text-and-cursor control the
    // titlebar's crumbs use, and none of the surface the mode group beside it wears.
    const trigger = wrapper.get('[data-testid="language-trigger"]');
    expect(trigger.classes()).toContain("text-menu-control");
    expect(trigger.classes()).not.toContain("document-mode-button");
    expect(language.classes()).not.toContain("document-mode-control");
    // And the list behind it is the menu the titlebar opens, the same one the worktree menu is.
    expect(wrapper.find(".marvis-menu").exists()).toBe(true);
    // The rows scroll, not the field that filters them.
    expect(wrapper.get('[role="listbox"]').classes()).toContain("marvis-menu-scroll");
    expect(wrapper.find(".marvis-menu-search").exists()).toBe(true);
    expect(wrapper.get('[role="option"]').classes()).toContain("menu-item");
    expect(mode.findAll("button").map((button) => button.text())).toEqual(["View", "Code"]);

    // A Markdown preview renders its own fences, so there is no grammar left to choose there.
    await wrapper.setProps({ mode: "view" });
    expect(wrapper.find('[data-testid="language-trigger"]').exists()).toBe(false);
    expect(wrapper.find('[aria-label="Document mode"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("forces a grammar onto a file whose extension says nothing about it", async () => {
    mocks.readCheckoutFile.mockResolvedValue({ path: "notes.txt", content: "def answer():\n    return 42" });
    const wrapper = mount(DocumentPane, { props: documentPaneProps("notes.txt") });
    await flushPromises();

    // Nothing is applied yet, and the toolbar says so instead of naming a grammar that is not on.
    expect(wrapper.get('[data-testid="language-trigger"]').text()).toBe("Auto (sin resaltado)");
    expect(wrapper.find(".shiki").exists()).toBe(false);

    const search = wrapper.get('input[aria-label="Search highlight languages"]');
    // A search reads the label, the grammar name, or the suffix a file wears.
    await search.setValue("yml");
    expect(languageRowLabels(wrapper)).toEqual(["Auto", "Texto plano", "YAML"]);

    await search.setValue("pyt");
    await wrapper.get('input[aria-label="Search highlight languages"]').trigger("keydown.down");
    await wrapper.get('input[aria-label="Search highlight languages"]').trigger("keydown.down");
    await wrapper.get('input[aria-label="Search highlight languages"]').trigger("keydown.enter");
    await vi.waitFor(() => expect(wrapper.findAll(".shiki .line")).toHaveLength(2));
    expect(wrapper.get('[data-testid="language-trigger"]').text()).toBe("Python");

    await search.setValue("zzz");
    // Auto and plain text are the ways back out, so a search never takes them away.
    expect(languageRowLabels(wrapper)).toEqual(["Auto", "Texto plano"]);
    expect(wrapper.get('[role="listbox"]').text()).toContain("No language matches");
    wrapper.unmount();
  });

  it("reads Auto as the extension says and can be put back to plain text", async () => {
    mocks.readCheckoutFile.mockResolvedValue({ path: "src/app.ts", content: "const answer: number = 42;" });
    const wrapper = mount(DocumentPane, { props: documentPaneProps("src/app.ts") });
    await vi.waitFor(() => expect(wrapper.findAll(".shiki .line")).toHaveLength(1));

    await pickLanguage(wrapper, "Texto plano");
    await flushPromises();
    expect(wrapper.find(".shiki").exists()).toBe(false);
    expect(wrapper.get('[data-testid="language-trigger"]').text()).toBe("Texto plano");

    await pickLanguage(wrapper, "Auto");
    await vi.waitFor(() => expect(wrapper.findAll(".shiki .line")).toHaveLength(1));
    expect(wrapper.get('[data-testid="language-trigger"]').text()).toBe("TypeScript");
    wrapper.unmount();
  });

  it("keeps each file's grammar to itself", async () => {
    mocks.readCheckoutFile.mockImplementation(async (_checkoutId: string, path: string) => ({
      path,
      content: path.endsWith(".ts") ? "const answer = 42;" : "def answer():\n    return 42",
    }));
    const wrapper = mount(DocumentPane, { props: documentPaneProps("notes.txt") });
    await flushPromises();
    await pickLanguage(wrapper, "Python");
    await vi.waitFor(() => expect(wrapper.find(".shiki .line").text()).toContain("def answer"));

    // The next file starts from its own extension, not from the last choice.
    await wrapper.setProps({ path: "src/app.ts" });
    await flushPromises();
    expect(wrapper.get('[data-testid="language-trigger"]').text()).toBe("TypeScript");
    await vi.waitFor(() => expect(wrapper.find(".shiki .line").text()).toContain("const answer"));

    // And the file that was forced keeps its grammar for as long as the window lives.
    await wrapper.setProps({ path: "notes.txt" });
    await flushPromises();
    expect(wrapper.get('[data-testid="language-trigger"]').text()).toBe("Python");
    await vi.waitFor(() => expect(wrapper.find(".shiki .line").text()).toContain("def answer"));
    wrapper.unmount();
  });

  it("highlights multiline source with Shiki and displays line numbers", async () => {
    mocks.readCheckoutFile.mockResolvedValue({
      path: "src/example.ts",
      content: "const answer: number = 42;\nreturn answer;",
    });
    const wrapper = mount(DocumentPane, { props: documentPaneProps("src/example.ts") });
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
    mocks.readCheckoutFile.mockResolvedValue({ path: "logs/output.txt", content: Array(5001).fill("line").join("\n") });
    const wrapper = mount(DocumentPane, { props: documentPaneProps("logs/output.txt") });
    await flushPromises();
    const source = wrapper.get('[aria-label="Source code"]');
    expect(source.findAll(".source-line-number")).toHaveLength(1);
    expect(source.get(".source-line-number").text().split("\n")).toHaveLength(5001);
    wrapper.unmount();
  });

  it("ignores an older file read after switching checkout and document", async () => {
    let resolveOld!: (value: { path: string; content: string }) => void;
    mocks.readCheckoutFile.mockImplementation((_checkoutId: string, path: string) => {
      if (path === "old.ts") return new Promise((resolve) => (resolveOld = resolve));
      return Promise.resolve({ path, content: "new checkout content" });
    });
    const wrapper = mount(DocumentPane, { props: documentPaneProps("old.ts") });
    await wrapper.setProps({
      checkout: checkout("checkout:new"),
      path: "new.ts",
      gitSnapshot: snapshot("checkout:new"),
    });
    await flushPromises();
    resolveOld({ path: "old.ts", content: "stale old checkout content" });
    await flushPromises();
    expect(wrapper.text()).toContain("new checkout content");
    expect(wrapper.text()).not.toContain("stale old checkout content");
    wrapper.unmount();
  });

  it("hides the previous file while the next one loads, and keeps it once loaded", async () => {
    mocks.readCheckoutFile.mockResolvedValue({ path: "a.ts", content: "const fileA = true;" });
    let resolveB!: (value: { path: string; content: string }) => void;
    mocks.readCheckoutFile.mockImplementation((_checkoutId: string, path: string) =>
      path === "a.ts"
        ? Promise.resolve({ path, content: "const fileA = true;" })
        : new Promise((resolve) => {
            resolveB = resolve;
          }),
    );
    const wrapper = mount(DocumentPane, { props: documentPaneProps("a.ts") });
    await vi.waitFor(() => expect(wrapper.text()).toContain("const fileA = true;"));

    await wrapper.setProps({ path: "b.ts" });
    await vi.waitFor(() => expect(wrapper.get('[role="status"]').text()).toBe("Loading file…"));
    // E.3: the file that was open is not what stays on screen.
    expect(wrapper.text()).not.toContain("const fileA = true;");

    resolveB({ path: "b.ts", content: "const fileB = true;" });
    await vi.waitFor(() => expect(wrapper.text()).toContain("const fileB = true;"));
    expect(wrapper.text()).not.toContain("const fileA = true;");
    wrapper.unmount();
  });

  it("refreshes a changed file while keeping its reading position", async () => {
    mocks.readCheckoutFile
      .mockResolvedValueOnce({ path: "src/app.ts", content: "const first = true;" })
      .mockResolvedValueOnce({ path: "src/app.ts", content: "const second = true;" });
    const wrapper = mount(DocumentPane, {
      props: { ...documentPaneProps("src/app.ts"), refreshRevision: 0 },
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

  it("keeps a file it cannot read on screen while the checkout reads it again", async () => {
    // The watcher re-reads the open file twice for every change anywhere in the checkout, and a
    // reason it cannot be read is not something to take away and hand back while that happens.
    mocks.readCheckoutFile.mockRejectedValue({ code: "binary_file", message: "not utf-8" });
    const wrapper = mount(DocumentPane, { props: documentPaneProps("assets/logo.png") });
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toBe("This file is binary or is not valid UTF-8.");

    let reRead!: (value: { path: string; content: string }) => void;
    mocks.readCheckoutFile.mockImplementation(
      () =>
        new Promise((resolve) => {
          reRead = resolve;
        }),
    );
    await wrapper.setProps({ refreshRevision: 1 });
    expect(wrapper.get('[role="alert"]').text()).toBe("This file is binary or is not valid UTF-8.");
    expect(wrapper.text()).not.toContain("Loading file");

    reRead({ path: "assets/logo.png", content: "readable now" });
    await flushPromises();
    expect(wrapper.text()).toContain("readable now");
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("leaves an unchanged document exactly where the reader left it", async () => {
    mocks.readCheckoutFile.mockResolvedValue({ path: "src/app.ts", content: "const same = true;" });
    const wrapper = mount(DocumentPane, {
      props: { ...documentPaneProps("src/app.ts"), refreshRevision: 0 },
    });
    await vi.waitFor(() => expect(wrapper.text()).toContain("const same = true;"));
    const source = wrapper.get('[aria-label="Source code"]').element;
    const viewport = wrapper.get('[aria-label="File contents"]');
    (viewport.element as HTMLElement).scrollTop = 96;
    await viewport.trigger("scroll");

    await wrapper.setProps({ refreshRevision: 1 });
    await flushPromises();
    expect(mocks.readCheckoutFile).toHaveBeenCalledTimes(2);
    // The same node, so nothing was taken down and built again, and the offset is still the
    // reader's: the bytes were the ones already on screen.
    expect(wrapper.get('[aria-label="Source code"]').element).toBe(source);
    expect(wrapper.text()).not.toContain("Highlighting source");
    expect((wrapper.get('[aria-label="File contents"]').element as HTMLElement).scrollTop).toBe(96);
    wrapper.unmount();
  });

  it("keeps the rendered Markdown page on screen while the file is read again", async () => {
    mocks.readCheckoutFile.mockResolvedValueOnce({
      path: "docs/readme.md",
      content: "![pic](pic.png)\n\n# First title",
    });
    const wrapper = mount(DocumentPane, {
      props: { ...documentPaneProps("docs/readme.md", "view"), refreshRevision: 0 },
    });
    await vi.waitFor(() => expect(wrapper.get(".markdown-preview").text()).toContain("First title"));

    let reRead!: (value: { path: string; content: string }) => void;
    let reImage!: (value: { mimeType: string; dataBase64: string; sizeBytes: number }) => void;
    mocks.readCheckoutFile.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          reRead = resolve;
        }),
    );
    mocks.readCheckoutMarkdownImage.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          reImage = resolve;
        }),
    );
    await wrapper.setProps({ refreshRevision: 1 });
    reRead({ path: "docs/readme.md", content: "![pic](pic.png)\n\n# Second title" });
    // The new page is being put together, image and all, so this is the window in which the one
    // the reader is on has to still be the one on screen.
    await vi.waitFor(() => expect(mocks.readCheckoutMarkdownImage).toHaveBeenCalledTimes(2));
    expect(wrapper.get(".markdown-preview").text()).toContain("First title");
    expect(wrapper.text()).not.toContain("Rendering preview");

    reImage({ mimeType: "image/png", dataBase64: "iVBORw0KGgo=", sizeBytes: 8 });
    await vi.waitFor(() => expect(wrapper.get(".markdown-preview").text()).toContain("Second title"));
    wrapper.unmount();
  });

  it("restores the Markdown reading position after relative images finish decoding", async () => {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, "decode");
    const decode = vi.fn(function (this: HTMLImageElement) {
      const viewport = this.closest(".document-pane")?.querySelector('[aria-label="File contents"]') as HTMLElement;
      viewport.scrollTop = 0;
      return Promise.resolve();
    });
    Object.defineProperty(HTMLImageElement.prototype, "decode", { configurable: true, value: decode });
    mocks.readCheckoutFile.mockResolvedValue({ path: "docs/readme.md", content: "![preview](image.png)" });
    const wrapper = mount(DocumentPane, {
      props: {
        ...documentPaneProps("docs/readme.md", "view"),
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

  it("does not read a file that is gone from the working tree", async () => {
    const gitSnapshot = snapshot("checkout:deleted", [{ path: "gone.ts", status: "D" }]);
    const wrapper = mount(DocumentPane, {
      props: documentPaneProps("gone.ts", "code", checkout("checkout:deleted"), gitSnapshot),
    });
    await flushPromises();
    expect(mocks.readCheckoutFile).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("This file was deleted");
    wrapper.unmount();
  });

  it("REPRO shows a readable file after an unreadable one", async () => {
    const view = ref<MainView>({ kind: "document", path: ".gitignore", mode: "code" });
    const currentCheckout = checkout("checkout:one");
    const gitSnapshot = snapshot(currentCheckout.id);
    mocks.listCheckoutFiles.mockResolvedValue({
      entries: [
        { name: ".DS_Store", path: ".DS_Store", kind: "file" },
        { name: ".gitignore", path: ".gitignore", kind: "file" },
        { name: "notes.md", path: "notes.md", kind: "file" },
      ],
      truncated: false,
    });
    mocks.readCheckoutFile.mockImplementation(async (_checkoutId: string, path: string) => {
      if (path === ".DS_Store") throw { code: "binary_file", message: "not utf-8" };
      return { path, content: `contents of ${path}\n` };
    });
    const harness = defineComponent({
      components: { InspectorPane, MainPane },
      setup() {
        function openFile(file: { checkoutId: string; path: string }) {
          view.value = { kind: "document", path: file.path, mode: isMarkdownPath(file.path) ? "view" : "code" };
        }
        return { view, currentCheckout, gitSnapshot, review: reviewApi(), openFile };
      },
      data: () => ({ repo }),
      template: `<div>
        <InspectorPane :checkout="currentCheckout" :repo="repo" :git-snapshot="gitSnapshot" @open-file="openFile" />
        <MainPane
          :checkout="currentCheckout"
          :view="view"
          :ready="true"
          :git-snapshot="gitSnapshot"
          :review="review"
          :active-session-id="null"
          :is-opening="false"
        />
      </div>`,
    });
    const wrapper = mount(harness, { global: { stubs: { SessionPane: true } } });
    await flushPromises();
    expect(wrapper.text()).toContain("contents of .gitignore");

    const fileRows = () => wrapper.get('[aria-label="Checkout files"]').findAll("button");
    await fileRows()[0].trigger("click");
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toBe("This file is binary or is not valid UTF-8.");

    // Back to the file that was already open before the unreadable one.
    await fileRows()[1].trigger("click");
    await flushPromises();
    // eslint-disable-next-line no-console
    console.log(
      "REPRO back to gitignore:",
      JSON.stringify(view.value),
      mocks.readCheckoutFile.mock.calls.map((call) => call[1]),
    );
    expect(wrapper.text()).toContain("contents of .gitignore");
    wrapper.unmount();
  });

  it("keeps the current file readable after its change is removed from Git status", async () => {
    const checkoutId = "checkout:cleaned";
    const gitSnapshot = snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]);
    mocks.readCheckoutFile.mockResolvedValue({ path: "src/app.ts", content: "const stillHere = true;" });
    const wrapper = mount(DocumentPane, {
      props: documentPaneProps("src/app.ts", "code", checkout(checkoutId), gitSnapshot),
    });
    await flushPromises();
    expect(wrapper.text()).toContain("const stillHere = true;");

    gitSnapshot.status!.files = [];
    gitSnapshot.statusRevision += 1;
    await flushPromises();
    expect(wrapper.text()).toContain("const stillHere = true;");
    expect(wrapper.text()).not.toContain("current file is unavailable");
    wrapper.unmount();
  });

  it("shows the whole change set as one diff per changed file", async () => {
    const checkoutId = "checkout:all-changes";
    const gitSnapshot = snapshot(checkoutId, [
      { path: "src/app.ts", status: "M", additions: 3, deletions: 1 },
      { path: "README.md", status: "A" },
    ]);
    mocks.getGitDiff.mockImplementation(async (_checkoutId: string, path: string) => ({
      path,
      patch: `diff --git a/${path} b/${path}\n@@ -1 +1 @@\n-old\n+new\n`,
      isBinary: false,
      large: false,
      tooLarge: false,
      totalLines: 3,
      hunks: [{ startLine: 0, endLine: 3, title: "@@ -1 +1 @@" }],
    }));
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot,
        review: reviewApi(),
        path: null,
        scrollTop: 0,
      },
    });
    await flushPromises();

    // Nothing is diffed until a file is opened: the backend pages one path at a time.
    expect(wrapper.get("header").text()).toContain("All changes");
    expect(wrapper.get("header").text()).toContain("feature");
    const headers = wrapper.findAll(".diff-file-header");
    expect(headers.map((header) => header.text())).toEqual(["Msrc/app.ts+3-1", "AREADME.md"]);
    expect(mocks.getGitDiff).not.toHaveBeenCalled();

    await headers[0]!.trigger("click");
    await vi.waitFor(() => expect(mocks.getGitDiff).toHaveBeenCalledWith(checkoutId, "src/app.ts"));
    expect(wrapper.findAll('[data-testid="diff-view"]')).toHaveLength(1);
    expect(mocks.getGitDiff).toHaveBeenCalledTimes(1);

    // A note made in a file of the change set belongs to that file, so the stack comments.
    const review = reviewApi();
    review.notes = [reviewNote({ id: "note:1", path: "src/app.ts", lineStart: 1 })];
    await wrapper.setProps({ review });
    await headers[1]!.trigger("click");
    await vi.waitFor(() => expect(mocks.getGitDiff).toHaveBeenCalledWith(checkoutId, "README.md"));
    expect(wrapper.text()).toContain("revisit this calculation");
    wrapper.unmount();
  });

  it("says the change set is loading, failing, or empty from the Git status", async () => {
    const checkoutId = "checkout:changes-status";
    const loading = snapshot(checkoutId);
    loading.statusState = "loading";
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot: loading,
        review: reviewApi(),
        path: null,
        scrollTop: 0,
      },
    });
    await flushPromises();
    expect(wrapper.get('[role="status"]').text()).toBe("Loading changes…");

    const failing = snapshot(checkoutId);
    failing.statusState = "error";
    failing.changesStatusError = "could not read the index";
    await wrapper.setProps({ gitSnapshot: failing });
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toBe("could not read the index");

    await wrapper.setProps({ gitSnapshot: snapshot(checkoutId) });
    await flushPromises();
    expect(wrapper.get('[role="status"]').text()).toBe("No changed files.");
    wrapper.unmount();
  });

  it("draws a diff that cannot be read in the panel, where it does not expire", async () => {
    const checkoutId = "checkout:diff-error";
    mocks.getGitDiff.mockRejectedValueOnce(new Error("git is not available"));
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot: snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]),
        review: reviewApi(),
        path: "src/app.ts",
        scrollTop: 0,
      },
    });
    await flushPromises();

    // Nothing else would be on screen, so the reason is drawn rather than left to a toast.
    expect(wrapper.get('[role="alert"]').text()).toBe("git is not available");
    expect(toasts.value).toHaveLength(0);
    wrapper.unmount();
  });

  it("pages a large diff lazily", async () => {
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
        scrollTop: 0,
      },
    });
    await vi.waitFor(() => expect(wrapper.text()).toContain("+new line"));
    expect(mocks.getGitDiffPage).toHaveBeenCalledWith(checkoutId, "large.txt", 0, 32);
    wrapper.unmount();
  });

  it("restores a diff position independently and reports later scrolling", async () => {
    const checkoutId = "checkout:diff-position";
    const wrapper = mount(FileDiff, {
      props: {
        checkout: checkout(checkoutId),
        gitSnapshot: snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]),
        review: reviewApi(),
        path: "src/app.ts",
        scrollTop: 72,
      },
    });
    await flushPromises();
    const viewport = wrapper.get('[aria-label="Diff contents"]');
    expect((viewport.element as HTMLElement).scrollTop).toBe(72);
    (viewport.element as HTMLElement).scrollTop = 144;
    await viewport.trigger("scroll");
    expect(wrapper.emitted("scrollPositionChanged")).toEqual([[144]]);
    wrapper.unmount();
  });

  it("feeds saved notes to the hunk renderer so they appear under their line", async () => {
    const checkoutId = "checkout:hunk-notes";
    const gitSnapshot = snapshot(checkoutId, [{ path: "src/app.ts", status: "M" }]);
    const note = reviewNote({ id: "note:1", lineStart: 1 });
    const review = reviewApi();
    review.notes = [note, reviewNote({ id: "note:2", path: "src/other.ts", lineStart: 5 })];
    const wrapper = mount(FileDiff, {
      props: { checkout: checkout(checkoutId), gitSnapshot, review, path: "src/app.ts", scrollTop: 0 },
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
      props: { checkout: checkout(checkoutId), gitSnapshot, review, path: "src/app.ts", scrollTop: 0 },
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
          scrollTop: 0,
        },
      });
      await vi.waitFor(() => expect(special.text()).toContain(result.message));
      special.unmount();
    }
  });
});
