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

/**
 * The path a terminal would need to open this file: the checkout's own directory, then the path
 * the tree lists. The checkout directory is canonical and the file's path is relative, so this is
 * the one place the two are joined, and it owns the separator: a checkout that arrives with a
 * trailing slash must not produce a doubled one.
 */
export function absoluteFilePath(canonicalPath: string, path: string): string {
  return `${canonicalPath.replace(/[\\/]+$/, "")}/${path}`;
}
