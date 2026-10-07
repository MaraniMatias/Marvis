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

export interface FileContent {
  path: string;
  content: string;
}

/**
 * A path a terminal printed that this checkout holds and the preview can open, as the
 * checkout-relative spelling the rest of the app asks for. Absent rather than a flag, so nothing
 * can be opened that the probe did not confirm.
 */
export interface FileProbe {
  path: string;
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
export function absoluteFilePath(
  canonicalPath: string,
  path: string,
  origin: "checkout" | "review" = "checkout",
  reviewRoot?: string,
): string {
  const root = origin === "review" ? reviewRoot : canonicalPath;
  if (!root) throw new Error("review file path requires its root");
  return `${root.replace(/[\\/]+$/, "")}/${path}`;
}
