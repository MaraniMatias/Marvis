import { invoke } from "@tauri-apps/api/core";
import type { Channel } from "@tauri-apps/api/core";
import type { OpenedFolder } from "../domain/folder";
import type { CheckoutImage, FileContent, FileSearchResult, FileTree } from "../domain/files";
import type { GitDiffPage, GitFileDiff, GitStatus } from "../domain/git";
import type { Session, TerminalLaunchType, TerminalSessionStatus, WorkspaceState } from "../domain/workspace";
import type { CreatedWorktree, RemovedWorktree, WorktreeDefaults, WorktreeRemovalInfo } from "../domain/worktree";
import type { CheckoutTerminalLayout } from "../domain/terminal-layout";
import type { AppLayoutState, CheckoutUiState } from "../domain/ui-state";

export interface CreatedTerminal {
  session: Session;
  workspace: WorkspaceState;
}

export interface EditorAvailability {
  zed: boolean;
  neovim: boolean;
}

export interface TerminalLaunchTarget {
  filePath: string;
  line: number;
  column?: number;
}

export function getEditorAvailability(): Promise<EditorAvailability> {
  return invoke<EditorAvailability>("editor_availability");
}

export function openInZed(checkoutId: string, filePath?: string, line?: number, column?: number): Promise<void> {
  return invoke<void>("editor_open_zed", {
    checkoutId,
    filePath: filePath ?? null,
    line: line ?? null,
    column: column ?? null,
  });
}

export function listCheckoutFiles(checkoutId: string, path: string): Promise<FileTree> {
  return invoke<FileTree>("files_list", { checkoutId, path });
}

export function searchCheckoutFiles(checkoutId: string): Promise<FileSearchResult> {
  return invoke<FileSearchResult>("files_search", { checkoutId });
}

export function readCheckoutFile(checkoutId: string, path: string): Promise<FileContent> {
  return invoke<FileContent>("file_read", { checkoutId, path });
}

export function readCheckoutMarkdownImage(
  checkoutId: string,
  markdownPath: string,
  imagePath: string,
): Promise<CheckoutImage> {
  return invoke<CheckoutImage>("file_read_markdown_image", { checkoutId, markdownPath, imagePath });
}

export function getGitStatus(checkoutId: string): Promise<GitStatus> {
  return invoke<GitStatus>("git_status", { checkoutId });
}

export function getGitDiff(checkoutId: string, path: string): Promise<GitFileDiff> {
  return invoke<GitFileDiff>("git_diff", { checkoutId, path });
}

export function getGitDiffPage(checkoutId: string, path: string, offset: number, limit: number): Promise<GitDiffPage> {
  return invoke<GitDiffPage>("git_diff_page", { checkoutId, path, offset, limit });
}

export function getGitViewedFiles(checkoutId: string): Promise<string[]> {
  return invoke<string[]>("git_viewed_files", { checkoutId });
}

export function markGitFileViewed(checkoutId: string, path: string): Promise<void> {
  return invoke<void>("git_mark_viewed", { checkoutId, path });
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

export function locateMissingCheckout(checkoutId: string, path: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("locate_missing_checkout", { checkoutId, path });
}

export function closeMissingCheckout(checkoutId: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("close_missing_checkout", { checkoutId });
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
  sessionType: TerminalLaunchType,
  onOutput: Channel<ArrayBuffer>,
  target?: TerminalLaunchTarget,
): Promise<CreatedTerminal> {
  return invoke<CreatedTerminal>("terminal_create", {
    request: {
      checkoutId,
      cols,
      rows,
      sessionType,
      filePath: target?.filePath ?? null,
      line: target?.line ?? null,
      column: target?.column ?? null,
    },
    onOutput,
  });
}

export function writeTerminal(checkoutId: string, sessionId: string, bytes: Uint8Array): Promise<void> {
  return invoke<void>("terminal_write", { checkoutId, sessionId, bytes: Array.from(bytes) });
}

export function resizeTerminal(checkoutId: string, sessionId: string, cols: number, rows: number): Promise<void> {
  return invoke<void>("terminal_resize", { checkoutId, sessionId, cols, rows });
}

export function getTerminalStatus(checkoutId: string, sessionId: string): Promise<TerminalSessionStatus> {
  return invoke<TerminalSessionStatus>("terminal_status", { checkoutId, sessionId });
}

export function closeTerminal(checkoutId: string, sessionId: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("terminal_close", { checkoutId, sessionId });
}

export function loadTerminalLayout(checkoutId: string): Promise<CheckoutTerminalLayout | null> {
  return invoke<CheckoutTerminalLayout | null>("terminal_layout_load", { checkoutId });
}

export function saveTerminalLayout(checkoutId: string, layout: CheckoutTerminalLayout): Promise<void> {
  return invoke<void>("terminal_layout_save", { checkoutId, layout });
}

export function loadAppLayout(): Promise<AppLayoutState> {
  return invoke<AppLayoutState>("ui_layout_load");
}

export function saveAppLayout(layout: AppLayoutState): Promise<void> {
  return invoke<void>("ui_layout_save", { layout });
}

export function loadCheckoutUiState(checkoutId: string): Promise<CheckoutUiState> {
  return invoke<CheckoutUiState>("checkout_ui_state_load", { checkoutId });
}

export function saveCheckoutUiState(checkoutId: string, state: CheckoutUiState): Promise<void> {
  return invoke<void>("checkout_ui_state_save", { checkoutId, state });
}

export function getWorktreeDefaults(checkoutId: string): Promise<WorktreeDefaults> {
  return invoke<WorktreeDefaults>("worktree_defaults", { checkoutId });
}

export function createWorktree(
  checkoutId: string,
  taskName: string,
  branch: string,
  location: string,
): Promise<CreatedWorktree> {
  return invoke<CreatedWorktree>("worktree_create", { checkoutId, taskName, branch, location });
}

export function getWorktreeRemovalInfo(checkoutId: string): Promise<WorktreeRemovalInfo> {
  return invoke<WorktreeRemovalInfo>("worktree_removal_info", { checkoutId });
}

export function removeWorktree(
  checkoutId: string,
  confirmDirty: boolean,
  confirmedDirtyFiles: string[],
  confirmedSessionIds: string[],
  expectedBranch: string | null,
  expectedUnmergedCommits: number,
  deleteBranch: boolean,
): Promise<RemovedWorktree> {
  return invoke<RemovedWorktree>("worktree_remove", {
    checkoutId,
    confirmation: {
      confirmDirty,
      confirmedDirtyFiles,
      confirmedSessionIds,
      expectedBranch,
      expectedUnmergedCommits,
      deleteBranch,
    },
  });
}
