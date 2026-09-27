export type FileEntryKind = "file" | "directory" | "symlink";

export interface FileEntry {
  name: string;
  path: string;
  kind: FileEntryKind;
  /**
   * A Git checkout ignores this file. It is still listed, and the tree shows it a step quieter
   * so a build directory reads as present but uninteresting. Only `.git` is ever withheld.
   */
  ignored?: boolean;
}

export interface FileTree {
  entries: FileEntry[];
  truncated: boolean;
}

export interface FileSearchResult {
  entries: FileEntry[];
  truncated: boolean;
}

export interface FileContent {
  path: string;
  content: string;
}

export interface CheckoutImage {
  mimeType: "image/png" | "image/jpeg" | "image/gif" | "image/webp";
  dataBase64: string;
  sizeBytes: number;
}
