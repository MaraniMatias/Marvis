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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
    /// The agent server for this checkout is not running and could not be started.
    AgentUnavailable,
    /// A session id does not belong to the checkout it was used against.
    AgentOwnershipMismatch,
    FileChanged,
    TerminalSessionMissing,
    TerminalOwnershipMismatch,
    InvalidTerminalDimensions,
    OperationFailed,
}
