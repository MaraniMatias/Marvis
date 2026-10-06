// @vitest-environment happy-dom
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { reactive } from "vue";
import type { GitStatus } from "../domain/git";
import { DEFAULT_CHECKOUT_UI_STATE } from "../domain/ui-state";
import type { CheckoutUiState } from "../domain/ui-state";
import type { Checkout, Repo } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { useToasts } from "../presentation/toasts";

const mocks = vi.hoisted(() => ({
  listCheckoutFiles: vi.fn(),
  getGitCheckoutDiffStats: vi.fn(),
  getGitDiffStats: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  listCheckoutFiles: mocks.listCheckoutFiles,
  getGitCheckoutDiffStats: mocks.getGitCheckoutDiffStats,
  getGitDiffStats: mocks.getGitDiffStats,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));

import InspectorPane from "./InspectorPane.vue";

const { toasts, dismiss } = useToasts();

/** Mirrors TREE_ROW_HEIGHT, TREE_WINDOW_SIZE and TREE_OVERSCAN: the window math is only right
 *  when the rendered rows are exactly this tall, so the test pins both sides. */
const ROW_HEIGHT = 22;
const WINDOW_SIZE = 64;
const OVERSCAN = 10;

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
    loading: false,
    statusState: status ? ("ready" as const) : ("error" as const),
    statusError: "",
    changesStatusError: "",
    changesWatchError: "",
    statusRevision: 0,
    statusEventRevision: 0,
    statusEventCheckoutId: null,
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
  checkout: Checkout | null;
  repo?: Repo;
  gitSnapshot?: ActiveGitSnapshot;
  savedState?: CheckoutUiState | null;
  fontScale?: number;
}) {
  return mount(InspectorPane, {
    props: { ...props, gitSnapshot: props.gitSnapshot ?? gitSnapshot(props.checkout?.id ?? "none") },
  });
}

/** The panel publishes its row height, so the test can do the window math with the real value. */
function publishedRowHeight(wrapper: ReturnType<typeof mountInspector>): number {
  const style = wrapper.get("aside").attributes("style") ?? "";
  return Number.parseFloat(/--tree-row-height:\s*([\d.]+)px/.exec(style)?.[1] ?? "0");
}

describe("InspectorPane", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.getGitCheckoutDiffStats.mockResolvedValue({});
    mocks.getGitDiffStats.mockResolvedValue([]);
    for (const toast of [...toasts.value]) dismiss(toast.id);
  });

  it("draws no edge of its own, so the handle that moves it is the only divider", () => {
    // A border down the panel's left, next to the five pixels of handle that move it, is one line
    // drawn twice. The panels meet on their own backgrounds instead, and the hairline the handle
    // draws is there only while the pointer is on it or a drag is in progress.
    const wrapper = mountInspector({ checkout: checkout("checkout:one") });

    expect(wrapper.get(".app-inspector").classes()).not.toContain("border-l");
    wrapper.unmount();
  });

  it("opens a selected file in the central document and marks the open row per checkout", async () => {
    mocks.listCheckoutFiles.mockImplementation(async (checkoutId: string) => ({
      entries: [{ name: `${checkoutId}.txt`, path: `${checkoutId}.txt`, kind: "file" }],
      truncated: false,
    }));
    const wrapper = mountInspector({ checkout: checkout("checkout:first") });
    await flushPromises();

    const first = wrapper.get('[aria-label="Checkout files"] button');
    expect(first.attributes("aria-current")).toBeUndefined();
    await first.trigger("click");
    expect(wrapper.emitted("openFile")).toEqual([[{ checkoutId: "checkout:first", path: "checkout:first.txt" }]]);
    // E.3: the open file is the row that says so.
    expect(first.classes()).toContain("is-selected");
    expect(first.attributes("aria-current")).toBe("true");

    await wrapper.setProps({ checkout: checkout("checkout:second") });
    await flushPromises();
    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    expect(wrapper.emitted("openFile")?.at(-1)).toEqual([
      { checkoutId: "checkout:second", path: "checkout:second.txt" },
    ]);

    await wrapper.setProps({ checkout: checkout("checkout:first") });
    await flushPromises();
    const restored = wrapper.get('[aria-label="Checkout files"] button');
    expect(restored.classes()).toContain("is-selected");
    expect(restored.attributes("aria-current")).toBe("true");
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
    // The root of the tree has no row of its own, so the panel itself says what failed: a toast
    // would expire and leave the tree empty with nothing to explain it.
    expect(wrapper.get('[role="alert"]').text()).toBe("Permission denied while reading this folder or file.");
    expect(toasts.value).toHaveLength(0);

    await wrapper.setProps({ checkout: checkout("gone", true) });
    expect(wrapper.text()).toContain("Checkout is missing.");
    wrapper.unmount();
  });

  it("says nothing under a folder that turned out to be empty", async () => {
    mocks.listCheckoutFiles.mockImplementation(async (_checkoutId: string, path: string) => {
      if (path === ".")
        return { entries: [{ name: "src", path: "src", kind: "directory" as const }], truncated: false };
      return { entries: [], truncated: false };
    });
    const wrapper = mountInspector({ checkout: checkout("hollow") });
    await flushPromises();

    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await flushPromises();

    // The folder stays and says nothing more.
    // A line of prose under every one of them is noise on a real tree.
    expect(wrapper.get('[aria-label="Checkout files"]').text()).not.toContain("Empty folder");
    expect(wrapper.findAll('[aria-label="Checkout files"] .file-row')).toHaveLength(1);
    wrapper.unmount();
  });

  it("quietens a dotfile once and a Git-ignored file twice, on the icon and the name together", async () => {
    mocks.listCheckoutFiles.mockResolvedValueOnce({
      entries: [
        { name: "index.ts", path: "index.ts", kind: "file" as const },
        { name: ".gitignore", path: ".gitignore", kind: "file" as const },
        { name: ".env", path: ".env", kind: "file" as const, ignored: true },
        { name: "build.log", path: "build.log", kind: "file" as const, ignored: true },
        { name: "dist", path: "dist", kind: "directory" as const, ignored: true },
      ],
      truncated: false,
    });
    const wrapper = mountInspector({ checkout: checkout("ramp") });
    await flushPromises();

    /** One row's two halves: the icon it draws with, and the name it writes beside it. */
    const row = (name: string) => {
      const found = wrapper.findAll(".file-row").find((candidate) => candidate.text().includes(name));
      if (!found) throw new Error(`no row for ${name}`);
      return {
        icon: found.get(".file-icon").classes(),
        label: found.get(".file-name").classes(),
      };
    };

    // The icon and the name step together, so a row is never quiet in only one of the two.
    expect(row("index.ts").icon).toContain("file-icon-normal");
    expect(row("index.ts").label).toContain("file-name-normal");
    expect(row(".gitignore").icon).toContain("file-icon-hidden");
    expect(row(".gitignore").label).toContain("file-name-hidden");
    expect(row("build.log").icon).toContain("file-icon-ignored");
    expect(row("build.log").label).toContain("file-name-ignored");

    // Being both a dotfile and ignored is one step down, not two: the ramp has three rungs and no
    // fourth, so the flag from the backend wins over the dot in the name.
    expect(row(".env").icon).toContain("file-icon-ignored");
    expect(row(".env").icon).not.toContain("file-icon-hidden");

    // A directory carries the ramp as well, which is what makes a build output read as present
    // but uninteresting instead of competing with the source next to it.
    expect(row("dist").icon).toContain("file-icon-ignored");
    wrapper.unmount();
  });

  it("keeps a folder that will not list on its own row", async () => {
    mocks.listCheckoutFiles.mockImplementation(async (_checkoutId: string, path: string) => {
      if (path === ".")
        return { entries: [{ name: "src", path: "src", kind: "directory" as const }], truncated: false };
      throw { code: "permission_denied", message: "denied" };
    });
    const wrapper = mountInspector({ checkout: checkout("folder") });
    await flushPromises();

    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await flushPromises();

    // The row says what failed, and it lasts as long as the row does.
    expect(wrapper.get('[aria-label="Checkout files"]').text()).toContain(
      "Permission denied while reading this folder or file.",
    );
    expect(toasts.value).toHaveLength(0);
    wrapper.unmount();
  });

  it("windows a large tree with the row height the row CSS is built from", async () => {
    const entries = Array.from({ length: 500 }, (_, index) => ({
      name: `file-${index}.txt`,
      path: `file-${index}.txt`,
      kind: "file" as const,
    }));
    mocks.listCheckoutFiles.mockResolvedValue({ entries, truncated: false });
    const wrapper = mountInspector({ checkout: checkout("large") });
    await flushPromises();

    const tree = wrapper.get('[aria-label="Checkout files"]');
    const rowHeight = publishedRowHeight(wrapper);
    expect(rowHeight).toBe(ROW_HEIGHT);
    expect(tree.findAll("button").length).toBe(WINDOW_SIZE);

    (tree.element as HTMLElement).scrollTop = 100 * rowHeight;
    await tree.trigger("scroll");
    const rows = tree.findAll("button");
    expect(rows.length).toBe(WINDOW_SIZE);
    // Window math and the rendered box agree: the first row is `scrollTop / height` minus the
    // overscan, and the padding above it is exactly that many rows tall.
    const first = 100 - OVERSCAN;
    expect(rows[0]!.text()).toContain(`file-${first}.txt`);
    expect(rows.at(-1)!.text()).toContain(`file-${first + WINDOW_SIZE - 1}.txt`);
    expect(tree.find("div[style]").attributes("style")).toContain(`padding-top: ${first * rowHeight}px`);

    (tree.element as HTMLElement).scrollTop = entries.length * rowHeight;
    await tree.trigger("scroll");
    expect(tree.text()).toContain("file-499.txt");
    expect(tree.text()).not.toContain("file-0.txt");
    wrapper.unmount();
  });

  it("scales the row height with the UI font size, and windows the list at the height it draws", async () => {
    const entries = Array.from({ length: 500 }, (_, index) => ({
      name: `file-${index}.txt`,
      path: `file-${index}.txt`,
      kind: "file" as const,
    }));
    mocks.listCheckoutFiles.mockResolvedValue({ entries, truncated: false });
    // 20px is `UI_FONT_SIZE_LIMITS.max`, so this is the largest row the preference can ask for.
    const wrapper = mountInspector({ checkout: checkout("large"), fontScale: 20 / 14 });
    await flushPromises();

    const tree = wrapper.get('[aria-label="Checkout files"]');
    const rowHeight = publishedRowHeight(wrapper);
    // A row pinned at the design height held the larger label in a line box too short for it.
    expect(rowHeight).toBeGreaterThan(ROW_HEIGHT);
    expect(rowHeight).toBe(Math.round(ROW_HEIGHT * (20 / 14)));

    (tree.element as HTMLElement).scrollTop = 100 * rowHeight;
    await tree.trigger("scroll");
    const first = 100 - OVERSCAN;
    const rows = tree.findAll("button");
    expect(rows[0]!.text()).toContain(`file-${first}.txt`);
    expect(tree.find("div[style]").attributes("style")).toContain(`padding-top: ${first * rowHeight}px`);
    wrapper.unmount();
  });

  it("keeps the tab keyboard navigation and sends a change selection to the main document", async () => {
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
    expect(sections.map((button) => button.text())).toEqual(["Files", "Changes 1"]);
    expect(sections[0]!.attributes("aria-controls")).toBe("inspector-panel-files");
    expect(sections[1]!.attributes("aria-controls")).toBe("inspector-panel-changes");
    expect(wrapper.get("#inspector-panel-files").attributes("aria-labelledby")).toBe("inspector-tab-files");
    expect(wrapper.get("#inspector-panel-files").attributes("tabindex")).toBe("0");
    expect(wrapper.get("#inspector-panel-changes").attributes("aria-labelledby")).toBe("inspector-tab-changes");

    await wrapper.get('[aria-label="Checkout files"] button').trigger("click");
    await wrapper.get('[aria-label="Inspector sections"]').findAll("button")[0]!.trigger("keydown", { key: "End" });
    await flushPromises();
    expect(wrapper.get("#inspector-tab-changes").attributes("aria-selected")).toBe("true");

    const changes = wrapper.get('[aria-label="Changed files"]');
    (changes.element as HTMLElement).scrollTop = 66;
    await changes.trigger("scroll");
    await flushPromises();
    expect(wrapper.emitted("updateUiState")?.at(-1)?.[0]).toMatchObject({ changesScrollTop: 66 });

    const change = changes.get("button.change-row");
    await change.trigger("click");
    expect(wrapper.emitted("openChange")).toEqual([[{ checkoutId: "git", path: "src/main.ts" }]]);
    expect(change.classes()).toContain("is-selected");
    expect(change.attributes("aria-current")).toBe("true");

    await wrapper.get(".new-item").trigger("click");
    expect(wrapper.emitted("openAllChanges")).toEqual([[{ checkoutId: "git" }]]);

    await wrapper.get("#inspector-tab-files").trigger("click");
    expect(wrapper.get('[aria-label="Checkout files"] button').classes()).toContain("is-selected");
    wrapper.unmount();
  });

  it("groups the changed files by parent directory", async () => {
    mocks.listCheckoutFiles.mockResolvedValue({ entries: [], truncated: false });
    const status: GitStatus = {
      branch: "feature",
      defaultBranch: "main",
      aheadCount: 1,
      files: [
        { path: "src/main.ts", status: "M" },
        { path: "src/lib/io.ts", status: "A" },
        { path: "old/name.ts", oldPath: "src/lib/old.ts", status: "R" },
        { path: "README.md", status: "D" },
        { path: "untracked.ts", status: "??" },
      ],
    };
    const wrapper = mountInspector({
      checkout: checkout("git"),
      repo,
      gitSnapshot: gitSnapshot("git", status),
      savedState: { ...DEFAULT_CHECKOUT_UI_STATE, inspectorTab: "changes" },
    });
    await flushPromises();

    const changes = wrapper.get('[aria-label="Changed files"]');
    // Groups follow the order the status reports its files, root files under "/".
    expect(changes.findAll(".details-group-header").map((header) => header.text())).toEqual([
      "src",
      "src/lib",
      "old",
      "/",
    ]);
    // Every changed file gets a row, status included: E.5 drops the `??` case from the tree
    // decorations, not from the list of what changed.
    const names = changes.findAll("button.change-row").map((row) => row.text());
    expect(names).toEqual(["Mmain.ts", "Aio.ts", "Rname.ts", "DREADME.md", "??untracked.ts"]);
    // A rename keeps its source path reachable without spending a second row on it.
    expect(changes.find("button.change-row[title='src/lib/old.ts → old/name.ts']").exists()).toBe(true);
    wrapper.unmount();
  });

  it("puts each file's own line counts on its change row", async () => {
    mocks.listCheckoutFiles.mockResolvedValue({ entries: [], truncated: false });
    mocks.getGitDiffStats.mockResolvedValue([
      { path: "src/main.ts", status: "M", additions: 12, deletions: 3 },
      { path: "README.md", status: "D", additions: 0, deletions: 8 },
      // Git has no count for a binary file, and the row draws nothing rather than "+0".
      { path: "assets/logo.png", status: "M" },
    ]);
    const status: GitStatus = {
      branch: "feature",
      defaultBranch: "main",
      aheadCount: 1,
      files: [
        { path: "src/main.ts", status: "M" },
        { path: "README.md", status: "D" },
        { path: "assets/logo.png", status: "M" },
      ],
    };
    const wrapper = mountInspector({
      checkout: checkout("git"),
      repo,
      gitSnapshot: gitSnapshot("git", status),
      savedState: { ...DEFAULT_CHECKOUT_UI_STATE, inspectorTab: "changes" },
    });
    await flushPromises();

    const changes = wrapper.get('[aria-label="Changed files"]');
    const rows = changes.findAll("button.change-row");
    expect(rows[0]!.get(".diff-add").text()).toBe("+12");
    expect(rows[0]!.get(".diff-del").text()).toBe("-3");
    // A zero addition has nothing to draw, and the deletion stands on its own.
    expect(rows[1]!.find(".diff-add").exists()).toBe(false);
    expect(rows[1]!.get(".diff-del").text()).toBe("-8");
    expect(rows[2]!.find(".diff-add").exists()).toBe(false);
    expect(rows[2]!.find(".diff-del").exists()).toBe(false);
    // The counts describe the listed files, not a different change set.
    expect(mocks.getGitDiffStats).toHaveBeenCalledWith("git");
    wrapper.unmount();
  });

  it("fetches every saved expanded directory, including nested paths", async () => {
    mocks.listCheckoutFiles.mockImplementation(async (_checkoutId: string, path: string) => ({
      entries:
        path === "."
          ? [{ name: "src", path: "src", kind: "directory" }]
          : path === "src"
            ? [{ name: "nested", path: "src/nested", kind: "directory" }]
            : [{ name: "main.ts", path: "src/nested/main.ts", kind: "file" }],
      truncated: false,
    }));
    const wrapper = mountInspector({
      checkout: checkout("nested"),
      savedState: {
        ...DEFAULT_CHECKOUT_UI_STATE,
        expandedDirectories: ["src", "src/nested"],
      },
    });
    await flushPromises();
    expect(mocks.listCheckoutFiles).toHaveBeenCalledWith("nested", ".");
    expect(mocks.listCheckoutFiles).toHaveBeenCalledWith("nested", "src");
    expect(mocks.listCheckoutFiles).toHaveBeenCalledWith("nested", "src/nested");
    const tree = wrapper.get('[aria-label="Checkout files"]');
    expect(tree.text()).toContain("main.ts");
    // The depth gutter is the mockup's: 6px plus 14px per level.
    expect(tree.findAll("button").map((row) => row.attributes("style"))).toEqual([
      "padding-left: 6px;",
      "padding-left: 20px;",
      "padding-left: 34px;",
    ]);
    await tree.findAll("button")[0]!.trigger("click");
    expect(wrapper.get('[aria-label="Checkout files"]').findAll("button")).toHaveLength(1);
    wrapper.unmount();
  });

  it("refreshes the file tree when the shared owner reports a Git status event", async () => {
    let resolveRefresh!: (value: {
      entries: { name: string; path: string; kind: "file" }[];
      truncated: boolean;
    }) => void;
    mocks.listCheckoutFiles
      .mockResolvedValueOnce({ entries: [{ name: "before.txt", path: "before.txt", kind: "file" }], truncated: false })
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveRefresh = resolve;
          }),
      );
    const snapshot = gitSnapshot("git");
    const wrapper = mountInspector({ checkout: checkout("git"), repo, gitSnapshot: snapshot });
    await flushPromises();
    expect(wrapper.text()).not.toContain("after.txt");

    snapshot.statusEventCheckoutId = "git";
    snapshot.statusEventRevision += 3;
    await vi.waitFor(() => expect(resolveRefresh).toBeTypeOf("function"));
    expect(wrapper.text()).toContain("before.txt");
    expect(wrapper.text()).not.toContain("Loading files");
    resolveRefresh({
      entries: [
        { name: "before.txt", path: "before.txt", kind: "file" },
        { name: "after.txt", path: "after.txt", kind: "file" },
      ],
      truncated: false,
    });
    await flushPromises();
    expect(wrapper.text()).toContain("after.txt");
    expect(mocks.listCheckoutFiles).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });
});
