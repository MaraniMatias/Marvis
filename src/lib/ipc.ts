import { invoke } from "@tauri-apps/api/core";
import type { Channel } from "@tauri-apps/api/core";
import type { OpenedFolder } from "../domain/folder";
import type { CheckoutImage, FileContent, FileSearchResult, FileTree } from "../domain/files";
import type { GitCheckoutDiffStats, GitDiffPage, GitFileDiff, GitFileDiffStats, GitStatus } from "../domain/git";
import type { ReviewNote, ReviewRound, ReviewSide } from "../domain/review";
import type { RecentPath, Session, TerminalSessionStatus, WorkspaceState } from "../domain/workspace";
import type { CreatedWorktree, RemovedWorktree, WorktreeDefaults, WorktreeRemovalInfo } from "../domain/worktree";
import type { CheckoutTerminalLayout } from "../domain/terminal-layout";
import type { AppLayoutState, CheckoutUiState } from "../domain/ui-state";
import type { AppSettings } from "../domain/settings";
import type { AgentAgent, AgentSession } from "../domain/agent";
import type { ReviewTarget } from "../presentation/review-notes";
import type { DocumentOrigin } from "../domain/main-document";

export interface CreatedTerminal {
  session: Session;
  workspace: WorkspaceState;
}

export function listCheckoutFiles(checkoutId: string, path: string): Promise<FileTree> {
  return invoke<FileTree>("files_list", { checkoutId, path });
}

export function searchCheckoutFiles(checkoutId: string): Promise<FileSearchResult> {
  return invoke<FileSearchResult>("files_search", { checkoutId });
}

export function readCheckoutFile(checkoutId: string, path: string, origin: DocumentOrigin): Promise<FileContent> {
  return invoke<FileContent>("file_read", { checkoutId, path, origin });
}

export function writeCheckoutFile(
  checkoutId: string,
  path: string,
  content: string,
  expectedContent: string,
  origin: DocumentOrigin,
): Promise<void> {
  return invoke<void>("file_write", { checkoutId, path, content, expectedContent, origin });
}

export function exportReviewMarkdown(date: string, timestamp: string, markdown: string): Promise<string> {
  return invoke<string>("review_export_markdown", { date, timestamp, markdown });
}

export function getReviewRootPath(): Promise<string> {
  return invoke<string>("review_root_path");
}

export function loadReviewTarget(checkoutId: string): Promise<ReviewTarget> {
  return invoke<ReviewTarget>("review_target_load", { checkoutId });
}

export function saveReviewTarget(checkoutId: string, target: ReviewTarget): Promise<void> {
  return invoke<void>("review_target_save", { checkoutId, target });
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

/** One call for the line counts of every registered Git checkout, so the sidebar does not
 *  have to ask checkout by checkout. The base ref is resolved in Rust, never sent from here. */
export function getGitCheckoutDiffStats(): Promise<GitCheckoutDiffStats> {
  return invoke<GitCheckoutDiffStats>("git_checkout_diff_stats");
}

/** The line counts of one checkout's changed files, against the same base ref the file list
 *  is built from. */
export function getGitDiffStats(checkoutId: string): Promise<GitFileDiffStats> {
  return invoke<GitFileDiffStats>("git_diff_stats", { checkoutId });
}

export function getGitDiff(checkoutId: string, path: string): Promise<GitFileDiff> {
  return invoke<GitFileDiff>("git_diff", { checkoutId, path });
}

export function getGitDiffPage(checkoutId: string, path: string, offset: number, limit: number): Promise<GitDiffPage> {
  return invoke<GitDiffPage>("git_diff_page", { checkoutId, path, offset, limit });
}

/** Watches a whole repository rather than one checkout. Its worktrees share a Git directory, so
 *  a commit in one of them moves the merge base every sibling counts its lines against. */
export function watchGitRepo(repoId: string): Promise<void> {
  return invoke<void>("git_watch_repo", { repoId });
}

export function listReviewNotes(checkoutId: string): Promise<ReviewNote[]> {
  return invoke<ReviewNote[]>("review_notes", { checkoutId });
}

export interface ReviewNoteCreate {
  checkoutId: string;
  path: string;
  side: ReviewSide;
  lineStart: number;
  lineEnd?: number;
  content: string;
  code: string;
}

export function createReviewNote(request: ReviewNoteCreate): Promise<ReviewNote> {
  return invoke<ReviewNote>("review_note_create", {
    request: {
      checkoutId: request.checkoutId,
      path: request.path,
      side: request.side,
      lineStart: request.lineStart,
      lineEnd: request.lineEnd ?? null,
      content: request.content,
      code: request.code,
    },
  });
}

export function updateReviewNote(checkoutId: string, id: string, content: string): Promise<ReviewNote> {
  return invoke<ReviewNote>("review_note_update", { checkoutId, id, content });
}

export function deleteReviewNote(checkoutId: string, id: string): Promise<void> {
  return invoke<void>("review_note_delete", { checkoutId, id });
}

export interface ReviewAnchorCheck {
  id: string;
  currentCode: string;
}

export function verifyReviewNoteAnchors(
  checkoutId: string,
  path: string,
  checks: ReviewAnchorCheck[],
): Promise<ReviewNote[]> {
  return invoke<ReviewNote[]>("review_note_anchors_verify", { request: { checkoutId, path, checks } });
}

export function clearReviewNoteOutdated(checkoutId: string, id: string): Promise<ReviewNote> {
  return invoke<ReviewNote>("review_note_outdated_clear", { checkoutId, id });
}

export function resolveReviewNote(checkoutId: string, id: string): Promise<ReviewNote> {
  return invoke<ReviewNote>("review_note_resolve", { checkoutId, id });
}

export function listReviewRounds(checkoutId: string): Promise<ReviewRound[]> {
  return invoke<ReviewRound[]>("review_rounds", { checkoutId });
}

/**
 * Records the round, then delivers it as one message. Never both or neither.
 *
 * With `queue` the round is stored and the agent is left alone: the send happens later,
 * from the stored message, when the caller flushes it.
 */
export function dispatchReviewRound(request: {
  checkoutId: string;
  sessionId: string;
  ids: string[];
  markdown: string;
  queue?: boolean;
}): Promise<ReviewRound> {
  return invoke<ReviewRound>("review_round_dispatch", { request });
}

/** Sends the rounds held back while the agent was busy. Returns how many went out. */
export function flushReviewRounds(checkoutId: string): Promise<number> {
  return invoke<number>("review_round_flush", { checkoutId });
}

export function requeueReviewRounds(checkoutId: string): Promise<number> {
  return invoke<number>("review_rounds_requeue", { checkoutId });
}

/** Asks the session whether a round's message arrived, and settles the round accordingly. */
export function reconcileReviewRound(checkoutId: string, roundId: string): Promise<ReviewRound> {
  return invoke<ReviewRound>("review_round_reconcile", { checkoutId, roundId });
}

export function ackReviewRound(checkoutId: string, roundId: string): Promise<ReviewRound> {
  return invoke<ReviewRound>("review_round_ack", { checkoutId, roundId });
}

export function listAgentSessions(checkoutId: string): Promise<AgentSession[]> {
  return invoke<AgentSession[]>("agent_sessions", { checkoutId });
}

/** Every agent the checkout's server offers, with the color OpenCode paints it with. */
export function listAgentAgents(checkoutId: string): Promise<AgentAgent[]> {
  return invoke<AgentAgent[]>("agent_agents", { checkoutId });
}

export function createAgentSession(checkoutId: string, title: string): Promise<AgentSession> {
  return invoke<AgentSession>("agent_session_create", { checkoutId, title });
}

export function stopAgent(checkoutId: string): Promise<void> {
  return invoke<void>("agent_stop", { checkoutId });
}

export function unwatchGitRepo(repoId: string): Promise<void> {
  return invoke<void>("git_unwatch_repo", { repoId });
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

/** Re-reads one repository's worktree list, for a worktree something other than this app added.
 *  `null` means the list did not move after all, so a caller already holding the workspace has
 *  nothing to apply. */
export function syncWorkspaceRepo(repoId: string): Promise<WorkspaceState | null> {
  return invoke<WorkspaceState | null>("sync_workspace_repo", { repoId });
}

/** The folders Marvis has had open before, newest first. The backend keeps the ten newest. */
export function listRecentPaths(): Promise<RecentPath[]> {
  return invoke<RecentPath[]>("list_recent_paths");
}

export function setDefaultBranch(repoId: string, branch: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("set_default_branch", { repoId, branch });
}

/** Takes a workdir off the panel, and only off the panel: the directory, its branch and its
 *  files are all left where they are, so opening the folder again brings the workdir back. */
export function closeCheckout(checkoutId: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("close_checkout", { checkoutId });
}

/** Removes a checkout whose directory is gone from Marvis alone: nothing on disk is deleted. */
export function closeMissingCheckout(checkoutId: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("close_missing_checkout", { checkoutId });
}

/** Takes a worktree off the panel and keeps it: `restoreArchivedWorktrees` brings it back. */
export function archiveCheckout(checkoutId: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("archive_checkout", { checkoutId });
}

/** Puts every worktree this repository archived back on the panel. */
export function restoreArchivedWorktrees(repoId: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("restore_archived_worktrees", { repoId });
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
  return invoke<CreatedTerminal>("terminal_create", {
    request: { checkoutId, cols, rows },
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

/** Names a session. The name is a label: it is trimmed, length-checked and never run. */
export function renameTerminal(checkoutId: string, sessionId: string, name: string): Promise<WorkspaceState> {
  return invoke<WorkspaceState>("terminal_rename", { checkoutId, sessionId, name });
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

/** The preferences the Settings dialog owns, from `~/.marvis/config.yml`. */
export function loadSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("settings_load");
}

export function saveSettings(settings: AppSettings): Promise<void> {
  return invoke<void>("settings_save", { settings });
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
