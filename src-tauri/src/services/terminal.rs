use std::{
    env,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    domain::workspace::{Session, SessionStatus, SessionType, WorkspaceState},
    persistence::{timestamp, Database},
    terminal::{OutputSink, SpawnOptions, TerminalBackend},
};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// Long enough for a branch-like name, short enough that a sidebar row never has to scroll
/// to show what a terminal is.
const MAX_NAME_CHARS: usize = 60;

pub struct CreatedTerminal {
    pub session: Session,
    pub workspace: WorkspaceState,
}

pub struct TerminalOptions {
    pub cols: u16,
    pub rows: u16,
    pub prompt: Option<String>,
}

pub fn create(
    database: &Database,
    backend: &TerminalBackend,
    checkout_id: &str,
    cols: u16,
    rows: u16,
    output: OutputSink,
) -> Result<CreatedTerminal, String> {
    create_with_options(
        database,
        backend,
        checkout_id,
        TerminalOptions {
            cols,
            rows,
            prompt: None,
        },
        output,
    )
}

pub fn create_with_options(
    database: &Database,
    backend: &TerminalBackend,
    checkout_id: &str,
    options: TerminalOptions,
    output: OutputSink,
) -> Result<CreatedTerminal, String> {
    let TerminalOptions { cols, rows, prompt } = options;
    let cwd = database.terminal_checkout_path(checkout_id)?;
    reject_prompt(prompt.as_deref())?;
    let program = inherited_shell();
    let name = program
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "shell".into());
    let session = Session {
        id: format!(
            "session:terminal:{}-{}-{}",
            timestamp(),
            std::process::id(),
            NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
        ),
        session_type: SessionType::Shell,
        checkout_id: checkout_id.to_string(),
        name,
        created_at: timestamp(),
        status: SessionStatus::Active,
    };

    let args = inherited_shell_args(&program);
    // Resolved before the spawn because `program` is moved into it below, and the hook text is a
    // property of the shell that is being started rather than of the PTY that is about to exist.
    let hook = shell_integration_hook(&program);
    backend.spawn(
        session.id.clone(),
        SpawnOptions {
            program,
            args,
            cwd,
            cols,
            rows,
        },
        output,
    )?;

    // Written after the spawn rather than before, because the hook is input and input needs a shell
    // that is there to read it. A failure here is deliberately swallowed: a shell that died on the
    // way up has nothing to instrument, and the session it failed to instrument is still a usable
    // terminal — only the sidebar bar stays blue, which is what a terminal without integration is.
    if let Some(hook) = hook {
        let _ = backend.write(&session.id, format!("{hook}\n").as_bytes());
    }

    match database.add_terminal_session(&session) {
        Ok(workspace) => Ok(CreatedTerminal { session, workspace }),
        Err(error) => {
            let _ = backend.close(&session.id);
            Err(error)
        }
    }
}

/// Renames a session, so a row the user cannot tell apart from its neighbours gets a name.
///
/// The name is a label and nothing more: it is never a path, an argument or a command, so the
/// only rules are that it is something to read and short enough for a sidebar row.
pub fn rename(database: &Database, session_id: &str, name: &str) -> Result<WorkspaceState, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a session needs a name".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "a session name is at most {MAX_NAME_CHARS} characters"
        ));
    }
    // Control characters would break the row they are drawn in, and a newline would draw a
    // second one. Neither is a name anyone means to type.
    if name.chars().any(char::is_control) {
        return Err("a session name cannot contain control characters".into());
    }
    database.rename_terminal_session(session_id, name)
}

pub fn close(
    database: &Database,
    backend: &TerminalBackend,
    session_id: &str,
) -> Result<WorkspaceState, String> {
    backend.close(session_id)?;
    database.remove_terminal_session(session_id)
}

fn inherited_shell() -> PathBuf {
    env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            ["/bin/zsh", "/bin/bash", "/bin/sh"]
                .into_iter()
                .map(PathBuf::from)
                .find(|path| path.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

fn inherited_shell_args(program: &Path) -> Vec<String> {
    let mut args = vec!["-l".into()];
    if program.file_name().and_then(|name| name.to_str()) == Some("zsh") {
        args.extend(["-o".into(), "AUTO_MENU".into()]);
    }
    args
}

/// The OSC 133 markers the hook below makes the shell print, spelled once.
///
/// `D;<code>` after every command is the shell's own `$?`, and `A` when the next one starts is what
/// tells the row to stop being red. OSC 133 is the ident every shell-integration script uses, and
/// xterm.js ships no handler for it, so nothing else contends for these bytes.
const OSC_EXIT: &str = r"\033]133;D;%s\007";
const OSC_STARTED: &str = r"\033]133;A\007";

/// The one line of shell input that makes this shell report how its commands ended.
///
/// Written as input to the PTY rather than added to a startup file, on purpose. Touching
/// `.zshrc`/`.zshenv`/`.zprofile` would edit something the user owns and that every other terminal
/// and app also reads, and pointing `ZDOTDIR` elsewhere would move the whole startup path: the test
/// `enables_native_zsh_menu_completion_and_leaves_startup_files_untouched` pins that those semantics
/// stay as they are. Input reaches this one shell and nothing else.
///
/// `None` means no integration: `sh` has no prompt hook that fires after a command with `$?` still
/// intact, and a terminal under it keeps today's behaviour rather than half a hook.
///
/// The line is echoed by the tty line discipline and stays in the scrollback. That is accepted: the
/// bytes written to the master are INPUT to the shell, so an ANSI erase would be eaten by zsh's ZLE
/// and ring the bell rather than reach the screen, and clearing `ECHO` on the master does not work
/// on macOS, where master and slave carry separate termios. One line per terminal, at the top of the
/// scrollback and before the first prompt, is what it costs.
fn shell_integration_hook(program: &Path) -> Option<String> {
    match program.file_name().and_then(|name| name.to_str()) {
        // `autoload -Uz add-zsh-hook` is not optional and `add-zsh-hook` is not a builtin. It is an
        // autoloadable function, so it exists only once something has loaded it — usually an rc file,
        // because frameworks like oh-my-zsh do. In a zsh with no rc files nothing has, and the call
        // below fails with `command not found: add-zsh-hook`: the line is already echoed by then, so
        // the shell looks fine, no hook is registered, and every session in that terminal silently
        // never reports an exit code. The bare `-U`/`-z` form is idempotent, so it costs nothing in
        // the rc-loaded case, and `add-zsh-hook` ships in zsh's own `fpath` on every platform this
        // app targets, which is what makes the autoload able to find it.
        Some("zsh") => Some(format!(
            "__marvis_precmd() {{ printf '{OSC_EXIT}' $?; }}; \
             autoload -Uz add-zsh-hook; \
             add-zsh-hook precmd __marvis_precmd; \
             __marvis_preexec() {{ printf '{OSC_STARTED}'; }}; \
             add-zsh-hook preexec __marvis_preexec"
        )),
        // A function rather than an inline `PROMPT_COMMAND` string because `$?` has to be read before
        // anything else in the command resets it, and an existing `PROMPT_COMMAND` is kept because
        // dropping it would take a user's own prompt work with it.
        Some("bash") => Some(format!(
            "__marvis_prompt_command() {{ printf '{OSC_EXIT}' \"$?\"; }}; \
             PROMPT_COMMAND=\"__marvis_prompt_command${{PROMPT_COMMAND:+; $PROMPT_COMMAND}}\"; \
             trap 'printf \"{OSC_STARTED}\"' DEBUG"
        )),
        _ => None,
    }
}

/// Terminals take no prompt any more: the review goes to the agent bridge, not a PTY.
fn reject_prompt(prompt: Option<&str>) -> Result<(), String> {
    match prompt {
        None => Ok(()),
        Some(_) => Err("a terminal session cannot carry a prompt".into()),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command, time::Duration};

    use tempfile::tempdir;

    use crate::{
        domain::workspace::Repo,
        persistence::Database,
        terminal::{synchronize_shell, wait_for_output, OutputSink, SpawnOptions, TerminalBackend},
    };

    use super::{
        create, create_with_options, inherited_shell_args, rename, shell_integration_hook,
        TerminalOptions,
    };

    fn plain_repo(path: &Path) -> Repo {
        Repo::plain(path, "now").unwrap()
    }

    #[test]
    fn enables_native_zsh_menu_completion_and_leaves_startup_files_untouched() {
        assert_eq!(
            inherited_shell_args(Path::new("/bin/zsh")),
            vec!["-l", "-o", "AUTO_MENU"]
        );
        assert_eq!(inherited_shell_args(Path::new("/bin/bash")), vec!["-l"]);
        if Command::new("zsh").arg("--version").output().is_err() {
            return;
        }

        let directory = tempdir().unwrap();
        let startup_files = [
            (".zshenv", "export MARVIS_ZSHENV=loaded\n"),
            (".zprofile", "export MARVIS_ZPROFILE=loaded\n"),
            (".zshrc", "export MARVIS_ZSHRC=loaded\n"),
        ];
        for (name, contents) in startup_files {
            fs::write(directory.path().join(name), contents).unwrap();
        }
        let before: Vec<_> = startup_files
            .iter()
            .map(|(name, _)| fs::read(directory.path().join(name)).unwrap())
            .collect();

        let args = inherited_shell_args(Path::new("zsh"));
        let output = Command::new("zsh")
            .args(&args)
            .args([
                "-i",
                "-c",
                "[[ -o AUTO_MENU && $MARVIS_ZSHENV == loaded && $MARVIS_ZPROFILE == loaded && $MARVIS_ZSHRC == loaded ]]",
            ])
            .env("HOME", directory.path())
            .env("ZDOTDIR", directory.path())
            .env("HISTFILE", directory.path().join(".zsh_history"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "zsh did not preserve menu completion/startup semantics: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for ((name, _), original) in startup_files.iter().zip(before) {
            assert_eq!(fs::read(directory.path().join(name)).unwrap(), original);
        }
    }

    /// Installs the hook in a real shell over a real PTY and asks a command to fail.
    ///
    /// Everything about this is a real shell rather than a stub because the whole subject is what a
    /// shell *accepts*: a hook line that is syntactically valid, calls functions that exist, and ends
    /// up registered. A test that only compared the string would have shipped the missing
    /// `autoload -Uz add-zsh-hook` straight through.
    fn assert_shell_reports_a_failing_command(shell: &str, args: &[&str], session: &str) {
        let Some(hook) = shell_integration_hook(Path::new(shell)) else {
            panic!("{shell} was expected to be integrated");
        };
        let backend = TerminalBackend::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        let output: OutputSink =
            Box::new(move |bytes| sender.send(bytes.to_vec()).map_err(|e| e.to_string()));
        backend
            .spawn(
                session.to_string(),
                SpawnOptions {
                    program: Path::new(shell).to_path_buf(),
                    args: args.iter().map(|arg| (*arg).to_string()).collect(),
                    cwd: std::env::current_dir().unwrap(),
                    cols: 80,
                    rows: 24,
                },
                output,
            )
            .unwrap();

        // The shell has to be at a prompt before the hook is typed at it, or the line lands in
        // whatever it is drawing instead.
        synchronize_shell(&backend, &receiver, session);
        backend
            .write(session, format!("{hook}\n").as_bytes())
            .unwrap();
        // The hook's own statements report a clean exit, so this wait also proves the hook was
        // accepted by the shell rather than typed into a broken line.
        wait_for_output(&receiver, b"\x1b]133;D;0\x07", Duration::from_secs(10));

        backend.write(session, b"false\n").unwrap();
        // `false` is the smallest command that fails, and the one every shell agrees on.
        wait_for_output(&receiver, b"\x1b]133;D;1\x07", Duration::from_secs(10));
        backend.close(session).unwrap();
    }

    /// A failed command is what the sidebar bar is for, and it is invisible to the backend: `clang`
    /// failing does not kill the shell, so no `waitpid` ever has a code to read. This is the only
    /// source of one — the shell's own `$?`, printed by the hook — so it runs the shells the app
    /// actually spawns and asserts on the bytes that come back.
    ///
    /// Every shell with no integration (`sh`) is skipped rather than faked, because a hook written for
    /// a shell that does not answer would make this pass for the wrong reason.
    #[test]
    fn a_failing_command_in_a_live_shell_reports_its_exit_code_over_osc_133() {
        for shell in ["/bin/zsh", "/bin/bash"] {
            if Command::new(shell).arg("--version").output().is_err() {
                continue;
            }
            assert_shell_reports_a_failing_command(shell, &["-l", "-i"], shell);
        }
    }

    /// The regression this pins: the hook used to call `add-zsh-hook` with nothing having loaded it.
    ///
    /// It is an autoloadable zsh *function*, not a builtin, so it only exists if something loaded it
    /// first — usually an rc file, because oh-my-zsh and friends do. Under `-l` on a developer's
    /// machine it therefore always worked, and the test above passed while the feature was silently
    /// dead for everyone without a zsh framework: `command not found: add-zsh-hook`, no hook
    /// registered, no OSC 133, ever, and no error anywhere because the line was already echoed.
    ///
    /// `zsh -f` is that user. `-f` skips every rc file, so whatever the machine running this has
    /// installed cannot be what makes the hook work — only what the hook line does for itself.
    #[test]
    fn the_zsh_hook_reports_a_failure_in_a_zsh_with_no_startup_files() {
        if Command::new("/bin/zsh").arg("--version").output().is_err() {
            return;
        }
        assert_shell_reports_a_failing_command("/bin/zsh", &["-f", "-i"], "session:zsh-no-rc");
    }

    #[test]
    fn the_zsh_hook_loads_add_zsh_hook_before_it_calls_it() {
        let hook = shell_integration_hook(Path::new("/bin/zsh")).unwrap();
        let autoload = hook.find("autoload -Uz add-zsh-hook").expect(
            "the zsh hook must autoload add-zsh-hook, which is an autoloadable function and not a builtin",
        );
        let first_call = hook
            .find("add-zsh-hook precmd")
            .expect("the hook must install a precmd hook");
        assert!(
            autoload < first_call,
            "add-zsh-hook is called before it is loaded, so the hook is never registered"
        );
    }

    #[test]
    fn integrates_only_the_shells_that_can_report_an_exit_code() {
        // `sh` has no prompt hook that fires with `$?` still intact, so it is left alone rather than
        // given a hook that would report something other than the command's own exit code.
        assert_eq!(shell_integration_hook(Path::new("/bin/sh")), None);
        assert_eq!(shell_integration_hook(Path::new("/usr/bin/fish")), None);
        for shell in ["/bin/zsh", "/bin/bash", "/opt/homebrew/bin/zsh"] {
            assert!(
                shell_integration_hook(Path::new(shell)).is_some(),
                "{shell} should be integrated"
            );
        }
    }

    #[test]
    fn creates_multiple_sessions_only_for_persisted_checkouts_and_forgets_them_on_restart() {
        let directory = tempdir().unwrap();
        let checkout = directory.path().join("checkout");
        let other_checkout = directory.path().join("other-checkout");
        fs::create_dir(&checkout).unwrap();
        fs::create_dir(&other_checkout).unwrap();
        let database_path = directory.path().join("workspace.sqlite3");
        let repo = plain_repo(&checkout);
        let other_repo = plain_repo(&other_checkout);
        let database = Database::open(&database_path).unwrap();
        database.register_plain_repo(repo.clone()).unwrap();
        database.register_plain_repo(other_repo.clone()).unwrap();
        let backend = TerminalBackend::default();
        let output: OutputSink = Box::new(|_| Ok(()));

        assert!(create(
            &database,
            &backend,
            "checkout:unknown",
            80,
            24,
            Box::new(|_| Ok(()))
        )
        .is_err());
        let first = create(&database, &backend, &repo.checkouts[0].id, 80, 24, output).unwrap();
        let second = create(
            &database,
            &backend,
            &repo.checkouts[0].id,
            80,
            24,
            Box::new(|_| Ok(())),
        )
        .unwrap();
        assert_ne!(first.session.id, second.session.id);
        assert_eq!(
            first.session.status,
            crate::domain::workspace::SessionStatus::Active
        );
        assert_eq!(second.workspace.repos[0].checkouts[0].sessions.len(), 2);
        assert_eq!(
            second.workspace.active_session_id.as_deref(),
            Some(second.session.id.as_str())
        );
        let other = create(
            &database,
            &backend,
            &other_repo.checkouts[0].id,
            80,
            24,
            Box::new(|_| Ok(())),
        )
        .unwrap();
        assert_eq!(other.session.checkout_id, other_repo.checkouts[0].id);
        assert_eq!(
            other.workspace.active_checkout_id.as_deref(),
            Some(other_repo.checkouts[0].id.as_str())
        );

        backend.close(&first.session.id).unwrap();
        backend.close(&second.session.id).unwrap();
        backend.close(&other.session.id).unwrap();
        drop(database);
        // The PTYs died with the processes above, so the reopened database keeps the checkouts
        // and none of the sessions.
        let restored = Database::open(database_path)
            .unwrap()
            .load_workspace()
            .unwrap();
        let restored_first = restored
            .repos
            .iter()
            .find(|item| item.id == repo.id)
            .unwrap();
        let restored_checkout = restored_first
            .checkouts
            .iter()
            .find(|checkout| checkout.id == repo.checkouts[0].id)
            .unwrap();
        assert!(restored_checkout.sessions.is_empty());
        let restored_other_repo = restored
            .repos
            .iter()
            .find(|item| item.id == other_repo.id)
            .unwrap();
        let restored_other = restored_other_repo.checkouts.first().unwrap();
        assert_eq!(restored_other.id, other_repo.checkouts[0].id);
        assert!(restored_other.sessions.is_empty());
    }

    #[test]
    fn a_rename_is_a_label_and_nothing_else() {
        let directory = tempdir().unwrap();
        let checkout = directory.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let database = Database::open(directory.path().join("workspace.sqlite3")).unwrap();
        let repo = plain_repo(&checkout);
        database.register_plain_repo(repo.clone()).unwrap();
        let session = crate::domain::workspace::Session {
            id: "session:rename".into(),
            session_type: crate::domain::workspace::SessionType::Shell,
            checkout_id: repo.checkouts[0].id.clone(),
            name: "zsh".into(),
            created_at: "now".into(),
            status: crate::domain::workspace::SessionStatus::Active,
        };
        database.add_terminal_session(&session).unwrap();

        // What the user typed around a name is not part of it.
        let renamed = rename(&database, &session.id, "  build logs \n").unwrap();
        assert_eq!(renamed.repos[0].checkouts[0].sessions[0].name, "build logs");

        // An empty name would leave a row with nothing on it to click.
        assert!(rename(&database, &session.id, "   ").is_err());
        // A newline would draw a second row inside the first.
        assert!(rename(&database, &session.id, "logs\nrm -rf /").is_err());
        // A name long enough to be a payload is refused, not truncated.
        assert!(rename(&database, &session.id, &"x".repeat(61)).is_err());
        assert!(rename(&database, &session.id, &"x".repeat(60)).is_ok());
        // A session the database does not hold is not renamed, whatever it is called.
        assert!(rename(&database, "session:unknown", "anything").is_err());
    }

    #[test]
    fn terminals_reject_a_prompt_because_the_review_goes_to_the_agent_bridge() {
        let directory = tempdir().unwrap();
        let checkout = directory.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let database = Database::open(directory.path().join("workspace.sqlite3")).unwrap();
        let repo = plain_repo(&checkout);
        database.register_plain_repo(repo.clone()).unwrap();
        let backend = TerminalBackend::default();

        assert!(create_with_options(
            &database,
            &backend,
            &repo.checkouts[0].id,
            TerminalOptions {
                cols: 80,
                rows: 24,
                prompt: Some("injected".into()),
            },
            Box::new(|_| Ok(())),
        )
        .is_err());
    }
}
