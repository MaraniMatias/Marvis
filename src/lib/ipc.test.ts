import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { OpenedFolder } from "../domain/folder";
import { isIpcError } from "../domain/ipc";
import {
  ackReviewRound,
  clearReviewNoteOutdated,
  closeTerminal,
  createAgentSession,
  createReviewNote,
  createTerminal,
  deleteReviewNote,
  dispatchReviewRound,
  listAgentSessions,
  listReviewNotes,
  listReviewRounds,
  reconcileReviewRound,
  requeueReviewRounds,
  resolveReviewNote,
  stopAgent,
  updateReviewNote,
  verifyReviewNoteAnchors,
  getEditorAvailability,
  getGitCheckoutDiffStats,
  getGitDiffPage,
  getGitDiffStats,
  loadTerminalLayout,
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

  it("uses the checkout-scoped diff page command", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ path: "src/file.ts", lines: [] });

    await getGitDiffPage("checkout:one", "src/file.ts", 320, 32);

    expect(invoke).toHaveBeenNthCalledWith(1, "git_diff_page", {
      checkoutId: "checkout:one",
      path: "src/file.ts",
      offset: 320,
      limit: 32,
    });
  });

  it("asks for diff counts with a checkout id and never a ref or a path", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce({ "checkout:one": { additions: 3, deletions: 1 } })
      .mockResolvedValueOnce([]);

    await getGitCheckoutDiffStats();
    await getGitDiffStats("checkout:one");

    // The all-checkouts call carries no argument at all: the base ref is resolved in Rust.
    expect(invoke).toHaveBeenNthCalledWith(1, "git_checkout_diff_stats");
    expect(invoke).toHaveBeenNthCalledWith(2, "git_diff_stats", { checkoutId: "checkout:one" });
  });
});

describe("Review note IPC client", () => {
  beforeEach(() => vi.clearAllMocks());

  it("sends checkout-scoped review note commands", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await listReviewNotes("checkout:one");
    await createReviewNote({
      checkoutId: "checkout:one",
      path: "src/foo.js",
      side: "new",
      lineStart: 10,
      lineEnd: 12,
      content: "revisit this calculation",
      code: "const result = a + b;",
    });
    await updateReviewNote("checkout:one", "note:1", "edited");
    await deleteReviewNote("checkout:one", "note:1");

    expect(invoke).toHaveBeenNthCalledWith(1, "review_notes", { checkoutId: "checkout:one" });
    expect(invoke).toHaveBeenNthCalledWith(2, "review_note_create", {
      request: {
        checkoutId: "checkout:one",
        path: "src/foo.js",
        side: "new",
        lineStart: 10,
        lineEnd: 12,
        content: "revisit this calculation",
        code: "const result = a + b;",
      },
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "review_note_update", {
      checkoutId: "checkout:one",
      id: "note:1",
      content: "edited",
    });
    expect(invoke).toHaveBeenNthCalledWith(4, "review_note_delete", {
      checkoutId: "checkout:one",
      id: "note:1",
    });
  });

  it("sends checkout-scoped agent commands", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await listAgentSessions("checkout:one");
    await createAgentSession("checkout:one", "Review 2026-09-26");
    await stopAgent("checkout:one");

    expect(invoke).toHaveBeenNthCalledWith(1, "agent_sessions", { checkoutId: "checkout:one" });
    expect(invoke).toHaveBeenNthCalledWith(2, "agent_session_create", {
      checkoutId: "checkout:one",
      title: "Review 2026-09-26",
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "agent_stop", { checkoutId: "checkout:one" });
  });

  it("sends review round commands, which record before they send", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await listReviewRounds("checkout:one");
    await dispatchReviewRound({
      checkoutId: "checkout:one",
      sessionId: "ses_one",
      ids: ["note:1"],
      markdown: "# Code Review",
    });
    await requeueReviewRounds("checkout:one");
    await reconcileReviewRound("checkout:one", "round:1");
    await ackReviewRound("checkout:one", "round:1");
    await resolveReviewNote("checkout:one", "note:1");

    expect(invoke).toHaveBeenNthCalledWith(1, "review_rounds", { checkoutId: "checkout:one" });
    expect(invoke).toHaveBeenNthCalledWith(2, "review_round_dispatch", {
      request: {
        checkoutId: "checkout:one",
        sessionId: "ses_one",
        ids: ["note:1"],
        markdown: "# Code Review",
      },
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "review_rounds_requeue", { checkoutId: "checkout:one" });
    expect(invoke).toHaveBeenNthCalledWith(4, "review_round_reconcile", {
      checkoutId: "checkout:one",
      roundId: "round:1",
    });
    expect(invoke).toHaveBeenNthCalledWith(5, "review_round_ack", {
      checkoutId: "checkout:one",
      roundId: "round:1",
    });
    expect(invoke).toHaveBeenNthCalledWith(6, "review_note_resolve", {
      checkoutId: "checkout:one",
      id: "note:1",
    });
  });

  it("sends anchor checks and clears the outdated mark", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await verifyReviewNoteAnchors("checkout:one", "src/foo.js", [
      { id: "note:1", currentCode: "const result = a - b;" },
    ]);
    await clearReviewNoteOutdated("checkout:one", "note:1");

    expect(invoke).toHaveBeenNthCalledWith(1, "review_note_anchors_verify", {
      request: {
        checkoutId: "checkout:one",
        path: "src/foo.js",
        checks: [{ id: "note:1", currentCode: "const result = a - b;" }],
      },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "review_note_outdated_clear", {
      checkoutId: "checkout:one",
      id: "note:1",
    });
  });
});
