import { invoke } from "@tauri-apps/api/core";
import type { Channel } from "@tauri-apps/api/core";
import type { OpenedFolder } from "../domain/folder";
import type { FileContent, FileTree } from "../domain/files";
import type { GitFileDiff, GitStatus } from "../domain/git";
import type { Session, TerminalSessionStatus, WorkspaceState } from "../domain/workspace";

export interface CreatedTerminal {
  session: Session;
  workspace: WorkspaceState;
}

export function listCheckoutFiles(checkoutId: string, path: string): Promise<FileTree> {
  return invoke<FileTree>("files_list", { checkoutId, path });
}

export function readCheckoutFile(checkoutId: string, path: string): Promise<FileContent> {
  return invoke<FileContent>("file_read", { checkoutId, path });
}

export function getGitStatus(checkoutId: string): Promise<GitStatus> {
  return invoke<GitStatus>("git_status", { checkoutId });
}

export function getGitDiff(checkoutId: string, path: string): Promise<GitFileDiff> {
  return invoke<GitFileDiff>("git_diff", { checkoutId, path });
}

export function watchGitCheckout(checkoutId: string): Promise<void> {
  return invoke<void>("git_watch_checkout", { checkoutId });
}

export function unwatchGitCheckout(checkoutId: string): Promise<void> {
  return invoke<void>("git_unwatch_checkout", { checkoutId });
}

export function openFolder(path: string): Promise<OpenedFolder> {
  return invoke<OpenedFolder>("open_folder", { path });
}

export function restoreWorkspace(): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("restore_workspace");
}

export function registerFolder(path: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("register_folder", { path });
}

export function setDefaultBranch(repoId: string, branch: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("set_default_branch", { repoId, branch });
}

export function selectCheckout(checkoutId: string | null): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("select_checkout", { checkoutId });
}

export function selectSession(sessionId: string | null): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("select_session", { sessionId });
}

export function createTerminal(
  checkoutId: string,
  cols: number,
  rows: number,
  onOutput: Channel<ArrayBuffer>,
): Promise<CreatedTerminal> {
  return invoke<CreatedTerminal>("terminal_create", { checkoutId, cols, rows, onOutput });
}

export function writeTerminal(sessionId: string, bytes: Uint8Array): Promise<void> {
  return invoke<void>("terminal_write", { sessionId, bytes: Array.from(bytes) });
}

export function resizeTerminal(sessionId: string, cols: number, rows: number): Promise<void> {
  return invoke<void>("terminal_resize", { sessionId, cols, rows });
}

export function getTerminalStatus(sessionId: string): Promise<TerminalSessionStatus> {
  return invoke<TerminalSessionStatus>("terminal_status", { sessionId });
}

export function closeTerminal(sessionId: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("terminal_close", { sessionId });
}
