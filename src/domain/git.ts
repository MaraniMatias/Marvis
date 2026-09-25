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
  symlinkTarget?: string;
}
