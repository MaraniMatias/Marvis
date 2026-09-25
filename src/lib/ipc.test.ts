import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { OpenedFolder } from "../domain/folder";
import { isIpcError } from "../domain/ipc";
import { closeTerminal, createTerminal, openFolder, resizeTerminal, writeTerminal } from "./ipc";

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

describe("terminal IPC client", () => {
  beforeEach(() => vi.clearAllMocks());

  it("uses only the scoped terminal domain commands and sends binary input as bytes", async () => {
    const channel = { onmessage: null } as never;
    const bytes = new Uint8Array([0, 195, 169, 255]);
    vi.mocked(invoke).mockResolvedValue(undefined);

    await createTerminal("checkout:/work/repo", 96, 30, channel);
    await writeTerminal("session:one", bytes);
    await resizeTerminal("session:one", 97, 31);
    await closeTerminal("session:one");

    expect(invoke).toHaveBeenNthCalledWith(1, "terminal_create", {
      checkoutId: "checkout:/work/repo",
      cols: 96,
      rows: 30,
      onOutput: channel,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "terminal_write", {
      sessionId: "session:one",
      bytes: [0, 195, 169, 255],
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "terminal_resize", {
      sessionId: "session:one",
      cols: 97,
      rows: 31,
    });
    expect(invoke).toHaveBeenNthCalledWith(4, "terminal_close", { sessionId: "session:one" });
  });
});
