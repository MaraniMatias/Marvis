import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { OpenedFolder } from "../domain/folder";
import { isIpcError } from "../domain/ipc";
import {
  closeTerminal,
  createTerminal,
  getEditorAvailability,
  getGitDiffPage,
  getGitViewedFiles,
  loadTerminalLayout,
  markGitFileViewed,
  openFolder,
  openInZed,
  resizeTerminal,
  saveTerminalLayout,
  writeTerminal,
} from "./ipc";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("openFolder IPC client", () => {
  beforeEach(() => vi.clearAllMocks());

  it("sends the selected path to the narrow folder command", async () => {
    const folder: OpenedFolder = { path: "/tmp/test workspace", name: "test workspace" };
    vi.mocked(invoke).mockResolvedValue(folder);

    await expect(openFolder(folder.path)).resolves.toEqual(folder);
    expect(invoke).toHaveBeenCalledWith("open_folder", { path: folder.path });
  });

  it("recognizes only errors using the typed IPC error contract", () => {
    expect(isIpcError({ code: "folder_missing", message: "Folder not found" })).toBe(true);
    expect(isIpcError({ code: "arbitrary", message: "No" })).toBe(false);
  });
});

describe("editor IPC client", () => {
  beforeEach(() => vi.clearAllMocks());

  it("uses only the supported Zed command and preserves an exact file location", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ zed: true, neovim: false }).mockResolvedValueOnce(undefined);

    await expect(getEditorAvailability()).resolves.toEqual({ zed: true, neovim: false });
    await openInZed("checkout:/work/repo", "src/file; name.rs", 23, 8);

    expect(invoke).toHaveBeenNthCalledWith(1, "editor_availability");
    expect(invoke).toHaveBeenNthCalledWith(2, "editor_open_zed", {
      checkoutId: "checkout:/work/repo",
      filePath: "src/file; name.rs",
      line: 23,
      column: 8,
    });
  });
});

describe("terminal IPC client", () => {
  beforeEach(() => vi.clearAllMocks());

  it("uses only the scoped terminal domain commands and sends binary input as bytes", async () => {
    const channel = { onmessage: null } as never;
    const bytes = new Uint8Array([0, 195, 169, 255]);
    vi.mocked(invoke).mockResolvedValue(undefined);

    await createTerminal("checkout:/work/repo", 96, 30, "shell", channel);
    await writeTerminal("checkout:one", "session:one", bytes);
    await resizeTerminal("checkout:one", "session:one", 97, 31);
    await closeTerminal("checkout:one", "session:one");

    expect(invoke).toHaveBeenNthCalledWith(1, "terminal_create", {
      request: {
        checkoutId: "checkout:/work/repo",
        cols: 96,
        rows: 30,
        sessionType: "shell",
        filePath: null,
        line: null,
        column: null,
      },
      onOutput: channel,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "terminal_write", {
      checkoutId: "checkout:one",
      sessionId: "session:one",
      bytes: [0, 195, 169, 255],
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "terminal_resize", {
      checkoutId: "checkout:one",
      sessionId: "session:one",
      cols: 97,
      rows: 31,
    });
    expect(invoke).toHaveBeenNthCalledWith(4, "terminal_close", {
      checkoutId: "checkout:one",
      sessionId: "session:one",
    });
  });

  it("passes exact file locations only as Neovim launch data", async () => {
    const channel = { onmessage: null } as never;
    vi.mocked(invoke).mockResolvedValue(undefined);

    await createTerminal("checkout:/work/repo", 80, 24, "nvim", channel, {
      filePath: "src/file; name.rs",
      line: 23,
      column: 8,
    });

    expect(invoke).toHaveBeenCalledWith("terminal_create", {
      request: {
        checkoutId: "checkout:/work/repo",
        cols: 80,
        rows: 24,
        sessionType: "nvim",
        filePath: "src/file; name.rs",
        line: 23,
        column: 8,
      },
      onOutput: channel,
    });
  });

  it("loads and saves layout through checkout-scoped typed commands", async () => {
    const layout = {
      activeTabId: "tab-one",
      sessionOrder: ["session-one"],
      tabs: [{ id: "tab-one", root: { kind: "session" as const, sessionId: "session-one" } }],
    };
    vi.mocked(invoke).mockResolvedValueOnce(layout).mockResolvedValueOnce(undefined);

    await expect(loadTerminalLayout("checkout:one")).resolves.toEqual(layout);
    await saveTerminalLayout("checkout:one", layout);

    expect(invoke).toHaveBeenNthCalledWith(1, "terminal_layout_load", { checkoutId: "checkout:one" });
    expect(invoke).toHaveBeenNthCalledWith(2, "terminal_layout_save", { checkoutId: "checkout:one", layout });
  });
});

describe("Git review IPC client", () => {
  beforeEach(() => vi.clearAllMocks());

  it("uses checkout-scoped page and viewed-state commands", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce({ path: "src/file.ts", lines: [] })
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce(undefined);

    await getGitDiffPage("checkout:one", "src/file.ts", 320, 32);
    await getGitViewedFiles("checkout:one");
    await markGitFileViewed("checkout:one", "src/file.ts");

    expect(invoke).toHaveBeenNthCalledWith(1, "git_diff_page", {
      checkoutId: "checkout:one",
      path: "src/file.ts",
      offset: 320,
      limit: 32,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "git_viewed_files", { checkoutId: "checkout:one" });
    expect(invoke).toHaveBeenNthCalledWith(3, "git_mark_viewed", {
      checkoutId: "checkout:one",
      path: "src/file.ts",
    });
  });
});
