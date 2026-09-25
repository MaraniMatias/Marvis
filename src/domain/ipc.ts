export type IpcErrorCode =
  | "folder_missing"
  | "invalid_path"
  | "path_outside_checkout"
  | "not_repository"
  | "default_branch_unknown"
  | "invalid_checkout"
  | "checkout_ownership_mismatch"
  | "permission_denied"
  | "file_too_large"
  | "binary_file"
  | "git_failed"
  | "process_terminated"
  | "operation_failed";

export interface IpcError {
  code: IpcErrorCode;
  message: string;
}

export function isIpcError(value: unknown): value is IpcError {
  if (typeof value !== "object" || value === null) return false;

  const candidate = value as Partial<IpcError>;
  return (
    typeof candidate.code === "string" &&
    typeof candidate.message === "string" &&
    [
      "folder_missing",
      "invalid_path",
      "path_outside_checkout",
      "not_repository",
      "default_branch_unknown",
      "invalid_checkout",
      "checkout_ownership_mismatch",
      "permission_denied",
      "file_too_large",
      "binary_file",
      "git_failed",
      "process_terminated",
      "operation_failed",
    ].includes(candidate.code)
  );
}
