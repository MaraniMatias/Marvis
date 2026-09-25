use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: IpcErrorCode,
    pub message: String,
}

impl IpcError {
    pub fn new(code: IpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcErrorCode {
    FolderMissing,
    InvalidPath,
    PathOutsideCheckout,
    NotRepository,
    DefaultBranchUnknown,
    InvalidCheckout,
    CheckoutOwnershipMismatch,
    PermissionDenied,
    FileTooLarge,
    BinaryFile,
    GitFailed,
    ProcessTerminated,
    OperationFailed,
}
