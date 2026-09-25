export interface GitChangedFile {
  path: string;
  oldPath?: string;
  status: string;
}

export interface GitStatus {
  branch?: string;
  head?: string;
  defaultBranch: string;
  aheadCount: number;
  files: GitChangedFile[];
}

export interface GitFileDiff {
  path: string;
  patch: string;
  isBinary: boolean;
  large: boolean;
  tooLarge: boolean;
  totalLines: number;
  hunks: GitDiffHunk[];
  symlinkTarget?: string;
}

export interface GitDiffHunk {
  startLine: number;
  endLine: number;
  title: string;
}

export type GitDiffLineKind = "hunk" | "added" | "removed" | "context" | "meta";

export interface GitDiffPageLine {
  index: number;
  kind: GitDiffLineKind;
  text: string;
  oldLineNumber: number | null;
  newLineNumber: number | null;
}

export interface GitDiffPage {
  path: string;
  startLine: number;
  totalLines: number;
  lines: GitDiffPageLine[];
}
