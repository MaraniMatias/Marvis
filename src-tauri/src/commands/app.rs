use std::sync::Arc;

use tauri::State;

use crate::{
    domain::ipc::{IpcError, IpcErrorCode},
    services::agent::AgentService,
    terminal::TerminalBackend,
};

/// Ends every agent server and every terminal, which is what leaves none of them behind.
///
/// Both are children of this app and both are meant to last as long as the window does. An
/// `opencode serve` with no parent holds its port for as long as it runs.
pub fn stop_children(agents: &AgentService, terminal: &TerminalBackend) {
    agents.stop_all();
    terminal.shutdown();
}

/// Ends the processes the app started, before the window closes.
///
/// The window is what decides when the app ends, and it decides it in the frontend, so the sweep
/// is asked for over the bridge rather than taken from an exit event: `RunEvent::ExitRequested` and
/// `RunEvent::Exit` are both delivered once the runtime has decided the process is going, and on a
/// window close neither of them arrives at all. Measured on macOS, the window closes and the process
/// ends with status 0 while no exit event reaches the app, which is how every agent server outlived
/// the quit it belonged to until a later launch swept it up.
#[tauri::command]
pub async fn app_prepare_exit(
    agents: State<'_, Arc<AgentService>>,
    terminal: State<'_, Arc<TerminalBackend>>,
) -> Result<(), IpcError> {
    let agents = Arc::clone(agents.inner());
    let terminal = Arc::clone(terminal.inner());
    // The sweep waits on children, which takes longer than a bridge call is meant to hold a thread
    // for, and the window is still up: nothing here needs the event loop to keep turning.
    tauri::async_runtime::spawn_blocking(move || stop_children(&agents, &terminal))
        .await
        .map_err(|error| {
            IpcError::new(
                IpcErrorCode::OperationFailed,
                format!("the sweep before exit did not finish: {error}"),
            )
        })
}
