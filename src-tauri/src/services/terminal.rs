use std::{
    env, fs,
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
    create_with_settings(
        database,
        backend,
        checkout_id,
        options,
        &None,
        &None,
        output,
    )
}

/// `shell_integration` is read from the settings file rather than passed from the frontend, so that
/// turning it off needs no IPC change at all and cannot be bypassed by a caller that forgot.
///
/// `None` means "no settings file was reachable", which is treated as the default: the feature ships
/// on, and a settings file that cannot be read is a settings dialog problem rather than a reason to
/// start every terminal uninstrumented.
///
/// `script_dir` is where the shell-integration script is written, and is `None` when this app has no
/// folder in the user's home to write it into — a terminal opened then is simply uninstrumented
/// rather than pointed at a file that is not there.
pub fn create_with_settings(
    database: &Database,
    backend: &TerminalBackend,
    checkout_id: &str,
    options: TerminalOptions,
    shell_integration: &Option<bool>,
    script_dir: &Option<PathBuf>,
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
    // Read here rather than at the prompt so that turning the setting off leaves every terminal that
    // is already open exactly as it was, and turning it on does not retrofit one.
    //
    // The script is written here, before the spawn, and not at launch: that is what makes turning the
    // setting on part-way through a session work, without the file having to exist for a setting that
    // was off when the app started. A failure is swallowed — a terminal with no markers is still a
    // usable terminal, which is the whole fallback.
    let startup_line = if shell_integration.unwrap_or(true) {
        install_shell_integration(&program, script_dir.as_deref())
    } else {
        None
    };
    backend.spawn(
        session.id.clone(),
        SpawnOptions {
            program,
            args,
            cwd,
            cols,
            rows,
            startup_line,
        },
        output,
    )?;

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

/// The OSC 133 markers the sourced script below makes the shell print, spelled once.
///
/// `D;<code>` after every command is the shell's own `$?`, and `A` when the next one starts is what
/// tells the row to stop being red. OSC 133 is the ident every shell-integration script uses, and
/// xterm.js ships no handler for it, so nothing else contends for these bytes.
///
/// `print -P` rather than `printf` for zsh, which takes the escapes as two characters where `printf`
/// needs four: `\e` against `\033` and `\a` against `\007`.
const OSC_EXIT: &str = r"\e]133;D;$?\a";
const OSC_STARTED: &str = r"\e]133;A\a";

/// The same two markers for bash, which has no `print` and so spells the escapes the long way.
const OSC_EXIT_BASH: &str = r"\033]133;D;%s\007";
const OSC_STARTED_BASH: &str = r"\033]133;A\007";

/// What the sourced script does to the screen before the shell redraws its prompt.
///
/// A clear rather than an erase of the row the setup line was drawn on, which is what this used to be
/// and which was not enough. An erase removes the text and leaves the row, so a terminal whose prompt
/// is taller than one row opened with the prompt's first rows, then blank rows where the setup had
/// been, then the first command — a gap in the middle of the top of the screen, which is what was
/// reported. `ESC[3J` discards the scrollback, `ESC[H` homes the cursor and `ESC[2J` erases the
/// display, after which the prompt is drawn at row 0 and the terminal is what a freshly opened one
/// looks like.
///
/// Written as escapes rather than shelling out to `clear`, for two reasons that are both about not
/// depending on something that may not be there: `clear` is not guaranteed to be on `PATH`, and
/// neither is `tput`, and a setup step that fails is a terminal with no markers and an error in it.
///
/// It has to be the shell that emits this. Bytes written to the pty master are INPUT: zsh's line
/// editor would consume them, and a bare clear sent that way comes back as the visual bell rather
/// than as a clear. That was measured, and it is the whole reason the hook body lives in a file that
/// the shell sources instead of being typed at it.
///
/// ## Why wiping the screen is safe here
///
/// A clear run at the wrong moment destroys work, so the whole of the safety of this rests on one
/// ordering: when this runs, the shell cannot have read a byte of user input. That ordering is the
/// startup gate in `terminal/mod.rs`.
///
/// `TerminalBackend::write` — the only path user input takes to a pty — waits for the startup line to
/// go out before it writes anything, and it has no deadline of its own, so it cannot time out while
/// the line is still pending. `install_startup_line` writes the line and only then releases input, and
/// on its own timeout it abandons the line without writing it at all. So the screen at the instant
/// this script runs holds the shell's startup output, its prompt, and the setup line, and all three
/// are what a clear is for. The gate that exists to stop a keystroke overtaking the hook is the same
/// thing that makes this safe; neither is true without the other.
///
/// The gate used to carry a deadline of its own, and that was a hole in this argument rather than a
/// belt-and-braces measure: it was measured from the keystroke while the startup thread's was measured
/// from that thread's first scheduling, so a keystroke arriving in between would expire first, put the
/// user's command into the shell, and only then have the setup line written behind it. It has been
/// replaced by a backstop far beyond the startup thread's own timeout, which cannot fire while the
/// line is still pending. `a_keystroke_never_overtakes_the_startup_line` in `terminal/mod.rs` is the
/// test for the ordering this depends on.
///
/// ## What it costs
///
/// The startup output is real output somebody asked for: a toolchain banner, a version notice, a
/// warning from a startup file. With the integration on it is gone; with the integration off it is
/// there. That is a behavioural difference the feature introduces rather than hides, which is why it
/// is a setting, and why the setting says so.
const CLEAR_SCREEN: &str = r"\033[3J\033[H\033[2J";

/// The script a terminal sources, per shell.
///
/// Two bodies rather than one that dispatches, because they are genuinely different shells' work and
/// a `case` over `$0` inside a sourced file is a way for one of them to be wrong without the other
/// being noticed.
///
/// zsh appends to the hook arrays by name. `add-zsh-hook` is not used because it is not a builtin but
/// an autoloadable *function*, so it exists only once something has loaded it — usually an rc file,
/// because frameworks like oh-my-zsh do. In a zsh with no rc files nothing has, and calling it fails
/// with `command not found: add-zsh-hook`: the line is already echoed by then, so the shell looks
/// fine, no hook is registered, and every session in that terminal silently never reports an exit
/// code. Appending to the arrays is the whole of what `add-zsh-hook` does underneath, and
/// `precmd_functions` and `preexec_functions` are ordinary zsh parameters, so there is nothing to
/// load and nothing that can be missing.
///
/// Each entry is the *name* of a function rather than a command: zsh looks an entry up and runs it,
/// it does not eval it. That was measured, not assumed, and it is why the two definitions exist —
/// `precmd_functions+=('print -Pn "…"')` registers nothing and emits no marker at all.
///
/// ## bash: the two hooks, and what they do to a shell that already had them
///
/// bash gets a function rather than an inline `PROMPT_COMMAND` string because `$?` has to be read
/// before anything else in the command resets it. That is the whole reason for the function, and it
/// is why the function also **returns** the status it read: without the `return`, `printf`'s own exit
/// status — 0 — is what every prompt command after ours sees, and a user whose prompt colours on
/// `$?` gets a green prompt for a command that failed. Measured on both a bash 3.2 and a bash 5.3 in
/// a PTY: with the `return`, the commands after ours read the command's status; without it they read
/// 0.
///
/// Which shape the hook is installed in depends on what `PROMPT_COMMAND` already is, because bash
/// 5.1 made it an array and both are live in the wild:
///
/// - As an **array** it is prepended to: `PROMPT_COMMAND=(__marvis_prompt_command "${…[@]}")`. Each
///   element is then run with `$?` still holding the command's status, so ours reading it costs the
///   user's own elements nothing — measured, and that is the form a multi-element array needs or the
///   elements after the first are folded into one string and cannot be addressed individually.
/// - As a **string** it is prepended to as a string, which is what a bash older than 5.1 has and what
///   a user who set `PROMPT_COMMAND='a; b'` has.
///
/// Assigning a string to a variable that is already an array does *not* replace the array — it
/// assigns element 0 and leaves the rest, measured on both versions — which is why the original line
/// did not actually lose a user's prompt commands on bash 5.1+. What it did instead was fold element
/// 0 into a compound string (`[0]="__marvis_prompt_command; a"`), which is what a prompt framework
/// that later reads or rewrites `PROMPT_COMMAND` sees as a corrupted array. The array form removes
/// that, and the version test is there because on bash 3.2 an array `PROMPT_COMMAND` is *not* run as
/// an array at all — only element 0 is, which would silently drop every prompt command after the
/// first. That was measured too: `PROMPT_COMMAND=(a b)` runs `a` alone on 3.2 and `a` then `b` on
/// 5.3.
///
/// `trap … DEBUG` stands in for zsh's `preexec`, which bash has no named hook for. It replaces
/// whatever DEBUG trap the user had, and bash does not keep the old one anywhere — but it *does* print
/// it: `trap -p DEBUG` reports `trap -- '<body>' DEBUG`, and that is enough to chain it, since the
/// body is the trap's own quoted text and evaluating it re-arms exactly what was there. Verified
/// against bodies containing single quotes, semicolons and variable references. So the previous trap
/// is read back, stripped of the `trap -- ` prefix and the ` DEBUG` suffix, evaluated once to
/// unquote it, and run after the marker. `PS0` would avoid all of this — it is expanded before a
/// command runs and, measured in a real PTY through a real xterm.js render, its output is *not*
/// printed to the screen, so a marker emitted there is invisible. But `PS0` does not exist in bash
/// 3.2, which is what `/bin/bash` is on macOS and what this app spawns when `$SHELL` says so:
/// measured, a bash 3.2 with `PS0` set emits nothing at all for `cd`, `[ -f … ]` or `false`, where
/// the DEBUG trap fires for all three. Using it would silently drop the marker on the version of bash
/// most likely to be on the machine, so the DEBUG trap stays and the user's is chained instead.
///
/// One thing about the DEBUG trap that is *not* fixed here, because it is a property of bash 3.2
/// rather than of this script: a `trap` set inside a sourced file does not survive the sourcing
/// command on 3.2 — bash restores the trap state it saved when the `.` ran. Measured: on 3.2 the
/// user's DEBUG trap is what is armed after the hook is sourced, and no `A` marker is emitted at all;
/// on 5.3 the hook's is. So on bash 3.2 the chaining below is a no-op and the marker never fires,
/// which is the pre-existing behaviour of installing from a sourced file and not something this
/// script can fix from inside one.
fn shell_integration_script(program: &Path) -> Option<(&'static str, String)> {
    match program.file_name().and_then(|name| name.to_str()) {
        Some("zsh") => Some((
            "hook.zsh",
            format!(
                "# Written by Marvis and sourced by every terminal it opens. Nothing here is read by \
                 anything else, and it is safe to delete once those terminals are closed.\n\
                 #\n\
                 # The last line clears the screen, so this leaves nothing of itself behind and the \
                 terminal opens looking as though it had just been opened.\n\
                 marvis_pc() {{ print -Pn \"{OSC_EXIT}\"; }}\n\
                 precmd_functions+=(marvis_pc)\n\
                 marvis_px() {{ print -Pn \"{OSC_STARTED}\"; }}\n\
                 preexec_functions+=(marvis_px)\n\
                 printf '{CLEAR_SCREEN}'\n"
            ),
        )),
        Some("bash") => Some((
            "hook.bash",
            format!(
                "# Written by Marvis and sourced by every terminal it opens. Nothing here is read by \
                 anything else, and it is safe to delete once those terminals are closed.\n\
                 #\n\
                 # Every line here prepends to something the shell already had, because a bash \
                 that was configured before this file was sourced keeps its configuration either \
                 way.\n\
                 #\n\
                 # The last line clears the screen, so this leaves nothing of itself behind and the \
                 terminal opens looking as though it had just been opened.\n\
                 __marvis_prompt_command() {{ local __marvis_status=$?; printf '{OSC_EXIT_BASH}' \
                 \"$__marvis_status\"; return \"$__marvis_status\"; }}\n\
                 if [[ ${{BASH_VERSINFO[0]}} -gt 5 || ${{BASH_VERSINFO[0]}} -eq 5 && \
                 ${{BASH_VERSINFO[1]}} -ge 1 ]] && [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == \
                 'declare -a'* ]]; then\n\
                 \x20 PROMPT_COMMAND=(__marvis_prompt_command \"${{PROMPT_COMMAND[@]}}\")\n\
                 else\n\
                 \x20 PROMPT_COMMAND=\"__marvis_prompt_command${{PROMPT_COMMAND:+; $PROMPT_COMMAND}}\"\n\
                 fi\n\
                 __marvis_previous_debug=$(trap -p DEBUG)\n\
                 if [[ -n $__marvis_previous_debug ]]; then \
                 __marvis_previous_debug=${{__marvis_previous_debug#trap -- }}; \
                 __marvis_previous_debug=${{__marvis_previous_debug% DEBUG}}; eval \
                 \"__marvis_previous_debug=$__marvis_previous_debug\"; fi\n\
                 trap 'printf \"{OSC_STARTED_BASH}\"; eval \"$__marvis_previous_debug\"' DEBUG\n\
                 printf '{CLEAR_SCREEN}'\n"
            ),
        )),
        // `sh` has no prompt hook that fires after a command with `$?` still intact, and a terminal
        // under it keeps the behaviour it has today rather than half a hook.
        _ => None,
    }
}

/// The one line of input that installs the script, or `None` when there is nothing to install.
///
/// Kept to one short row on purpose. The erase above can only reach one row, and a line long enough
/// to wrap would leave its first row behind — which is how this was drawn as 146 characters wrapping
/// over three, and then as one line a person opened a terminal and did not want to look at.
fn shell_integration_line(script: &Path) -> String {
    // Quoted the way `TerminalSession.vue`'s `changeDirectory` quotes a path it types at a shell:
    // single quotes, with an embedded one closed, escaped and reopened. A home directory may contain
    // a space — this app's own folder is `~/.marvis`, but a person's `$HOME` is whatever they chose —
    // and an unquoted path is a terminal with no hook and an error nobody reads.
    format!(". '{}'", script.to_string_lossy().replace('\'', "'\\''"))
}

/// Writes the script if it is not already there, and returns the line that sources it.
///
/// The file is written before the spawn and is never deleted while a session is live, so it outlives
/// the shell that reads it: the shell is a separate process with its own cwd and environment, and the
/// only thing it needs from this one is that the file is on disk when it gets there. Nothing here
/// depends on this process still running.
///
/// Written per terminal rather than once at launch, because the setting is read per terminal: that
/// is what makes turning it on part-way through a session work without the file having to exist for a
/// setting that was off when the app started. Rewriting is avoided by comparing first, so opening a
/// dozen terminals touches the disk once.
fn install_shell_integration(program: &Path, script_dir: Option<&Path>) -> Option<String> {
    let (name, contents) = shell_integration_script(program)?;
    let dir = script_dir?;
    let script = dir.join(name);
    let already_there = fs::read_to_string(&script).is_ok_and(|existing| existing == contents);
    if !already_there {
        if let Err(error) = fs::create_dir_all(dir) {
            log::warn!(
                "terminal {} could not be integrated: {error}",
                program.display()
            );
            return None;
        }
        let written = crate::services::files::atomic_write(
            &script,
            contents.as_bytes(),
            script_permissions(&script),
        );
        if let Err(error) = written {
            log::warn!(
                "terminal {} could not be integrated: {}",
                program.display(),
                error.message
            );
            return None;
        }
    }
    Some(shell_integration_line(&script))
}

/// The mode the script is written with, or the one it already has.
///
/// Private to its owner, like `config.yml` and for the same reason: the script is code this app asks
/// somebody's shell to run, so it is not a file to leave readable by anyone else. One the person
/// chose is left alone, because a write is not a reason to undo a decision made outside this app.
fn script_permissions(script: &Path) -> fs::Permissions {
    if let Ok(metadata) = fs::metadata(script) {
        return metadata.permissions();
    }
    private_permissions()
}

#[cfg(unix)]
fn private_permissions() -> fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    fs::Permissions::from_mode(0o600)
}

#[cfg(not(unix))]
fn private_permissions() -> fs::Permissions {
    fs::Permissions::default()
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
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::Arc,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use crate::{
        domain::workspace::Repo,
        persistence::Database,
        terminal::{wait_for_output, OutputSink, SpawnOptions, TerminalBackend},
    };

    use super::{
        create_with_options, create_with_settings, inherited_shell_args, install_shell_integration,
        rename, shell_integration_line, shell_integration_script, TerminalOptions, CLEAR_SCREEN,
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

    /// Where the sourced script is written for a test, and the line that sources it.
    ///
    /// The real directory name is used where the test is about a real path, and a space is put in it
    /// by the caller that wants one, because a `$HOME` with a space in it is a person's machine rather
    /// than an edge case and the quoting has to survive it.
    fn script_in(directory: &Path, shell: &str) -> String {
        let line = install_shell_integration(Path::new(shell), Some(directory))
            .unwrap_or_else(|| panic!("{shell} was expected to be integrated"));
        line
    }

    /// Spawns a real shell with the real script, through the real spawn path, and asks a command to
    /// fail.
    ///
    /// Everything about this is a real shell rather than a stub because the whole subject is what a
    /// shell *accepts*: a script that is syntactically valid, calls things that exist, and ends up
    /// registered. A test that only compared the string would have shipped both the missing
    /// `autoload -Uz add-zsh-hook` and an `add-zsh-hook` that nothing had loaded.
    ///
    /// The line goes in as `startup_line` rather than being written by the test, so this exercises the
    /// same timing production does: the backend decides when the shell is ready for it.
    fn assert_shell_reports_a_failing_command(shell: &str, args: &[&str], session: &str) {
        assert_script_installs(shell, args, session, tempdir().unwrap().path());
    }

    /// Opens a real shell with the script written where `script_dir` says, and checks that a failing
    /// command is reported.
    fn assert_script_installs(shell: &str, args: &[&str], session: &str, script_dir: &Path) {
        let line = script_in(script_dir, shell);
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
                    startup_line: Some(line),
                },
                output,
            )
            .unwrap();

        // The script's own last command reports a clean exit, so this wait also proves the shell read
        // the file rather than being handed a line it could not find.
        wait_for_output(&receiver, b"\x1b]133;D;0\x07", Duration::from_secs(15));

        backend.write(session, b"false\n").unwrap();
        // `false` is the smallest command that fails, and the one every shell agrees on.
        wait_for_output(&receiver, b"\x1b]133;D;1\x07", Duration::from_secs(10));
        backend.close(session).unwrap();
    }

    /// A failed command is what the sidebar bar is for, and it is invisible to the backend: `clang`
    /// failing does not kill the shell, so no `waitpid` ever has a code to read. This is the only
    /// source of one — the shell's own `$?`, printed by the script — so it runs the shells the app
    /// actually spawns and asserts on the bytes that come back.
    ///
    /// Every shell with no integration (`sh`) is skipped rather than faked, because a script written
    /// for a shell that does not answer would make this pass for the wrong reason.
    #[test]
    fn a_failing_command_in_a_live_shell_reports_its_exit_code_over_osc_133() {
        for shell in ["/bin/zsh", "/bin/bash"] {
            if Command::new(shell).arg("--version").output().is_err() {
                continue;
            }
            assert_shell_reports_a_failing_command(shell, &["-l", "-i"], shell);
        }
    }

    /// The regression this pins: the script used to call `add-zsh-hook` with nothing having loaded it.
    ///
    /// It is an autoloadable zsh *function*, not a builtin, so it only exists if something loaded it
    /// first — usually an rc file, because oh-my-zsh and friends do. Under `-l` on a developer's
    /// machine it therefore always worked, and the test above passed while the feature was silently
    /// dead for everyone without a zsh framework: `command not found: add-zsh-hook`, no hook
    /// registered, no OSC 133, ever, and no error anywhere because the line was already echoed.
    ///
    /// `zsh -f` is that user. `-f` skips every rc file, so whatever the machine running this has
    /// installed cannot be what makes the hook work — only what the script does for itself. It now
    /// appends to `precmd_functions` directly, which is what `add-zsh-hook` does underneath, so there
    /// is no longer anything that can be missing.
    #[test]
    fn the_zsh_script_reports_a_failure_in_a_zsh_with_no_startup_files() {
        if Command::new("/bin/zsh").arg("--version").output().is_err() {
            return;
        }
        assert_script_installs(
            "/bin/zsh",
            &["-f", "-i"],
            "session:zsh-no-rc",
            tempdir().unwrap().path(),
        );
    }

    /// A `$HOME` with a space in it is somebody's machine, and an unquoted path there is a terminal
    /// with no hook and an error nobody reads.
    ///
    /// Live rather than string-compared because the whole failure is what the *shell* makes of the
    /// quoting: a path with a space in it, sourced for real, has to report a failing command.
    #[test]
    fn a_path_with_a_space_in_it_still_installs_the_script() {
        if Command::new("/bin/zsh").arg("--version").output().is_err() {
            return;
        }
        let directory = tempdir().unwrap();
        let with_a_space = directory.path().join("a folder of mine");
        fs::create_dir(&with_a_space).unwrap();
        assert_script_installs("/bin/zsh", &["-l", "-i"], "session:spaced", &with_a_space);
    }

    /// A quote in the path is the other half of the same quoting rule: single quotes are closed,
    /// escaped and reopened, the way `TerminalSession.vue`'s `changeDirectory` does it.
    #[test]
    fn a_path_with_a_quote_in_it_is_quoted_the_way_a_shell_needs() {
        let line = shell_integration_line(Path::new("/home/someone/it's mine/hook.zsh"));
        assert_eq!(
            line, ". '/home/someone/it'\\''s mine/hook.zsh'",
            "the quote must be closed, escaped and reopened, and the whole path wrapped once"
        );
        // And the line is still one row, which is what lets the erase reach it.
        assert!(!line.contains('\n'));
    }

    /// What a zsh script has to be, and is not allowed to be, anything else.
    ///
    /// `precmd_functions` takes the *name* of a function and runs it; it does not eval its entries.
    /// That was measured rather than assumed, and the difference is the whole reason the script
    /// defines two functions: `precmd_functions+=('print -Pn "…"')` is accepted without complaint,
    /// registers nothing, and emits no marker at all, which is the silent-no-op failure this feature
    /// cannot afford. `add-zsh-hook` is gone for the same class of reason — it is an autoloadable
    /// function that a shell with no rc files has never loaded.
    #[test]
    fn the_zsh_script_appends_function_names_and_needs_nothing_loaded_first() {
        let (_, script) = shell_integration_script(Path::new("/bin/zsh")).unwrap();
        assert!(
            !script.contains("add-zsh-hook"),
            "add-zsh-hook is an autoloadable function, and a shell with no rc files has not loaded it"
        );
        for parameter in ["precmd_functions+=(", "preexec_functions+=("] {
            let entry = script
                .split(parameter)
                .nth(1)
                .and_then(|rest| rest.split(')').next())
                .unwrap_or_default();
            let name = entry.trim();
            assert!(
                name.starts_with("marvis_"),
                "{parameter} must name a function, and got {entry:?}"
            );
            assert!(
                script.contains(&format!("{name}()")),
                "{name} is appended but never defined, so zsh would look it up and find nothing"
            );
        }
    }

    /// The clear has to come from the shell, and it has to be its last act.
    ///
    /// Bytes written to the pty master are input: zsh's line editor would consume them, and a bare
    /// clear sent that way comes back as the visual bell rather than as a clear. That was measured,
    /// and it is why the hook body lives in a file the shell sources.
    #[test]
    fn the_script_clears_the_screen_and_does_it_last() {
        for shell in ["/bin/zsh", "/bin/bash"] {
            let Some((_, script)) = shell_integration_script(Path::new(shell)) else {
                continue;
            };
            let mut lines = script
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>();
            let last = lines.pop().unwrap();
            assert_eq!(
                last.trim(),
                format!("printf '{CLEAR_SCREEN}'"),
                "the clear must be the last thing {shell} does, or output is drawn over it"
            );
            // Scrollback, home, display — in that order. Homing between the two erases is what puts
            // `2J` over the whole screen rather than over what is below the cursor, and `3J` is what
            // stops the setup line sitting in the scrollback a scrollback would bring back.
            assert_eq!(CLEAR_SCREEN, r"\033[3J\033[H\033[2J");
            // It is emitted as escapes rather than by running `clear`, which is not guaranteed to be
            // on PATH, and a setup step that fails is a terminal with no markers and an error in it.
            assert!(
                !script.contains("clear "),
                "the clear must not depend on a program being there"
            );
        }
    }

    /// A screen that has been wiped is a screen somebody could have lost work on, so the claim that it
    /// is safe is a claim about ordering and it is worth pinning the two halves of it here.
    ///
    /// The gate that holds user input back is in `terminal/mod.rs` and is tested there
    /// (`a_keystroke_never_overtakes_the_startup_line`). What is asserted here is that this script is
    /// the thing that depends on it: the clear and the gate are two ends of one guarantee, and a
    /// change to either without the other is how the guarantee is lost.
    #[test]
    fn the_clear_is_only_reachable_behind_the_input_gate() {
        for shell in ["/bin/zsh", "/bin/bash"] {
            let Some((_, script)) = shell_integration_script(Path::new(shell)) else {
                continue;
            };
            // The script reaches the screen only as the last statement of the one line that is gated,
            // so there is no path from this file to the screen that does not go through it.
            assert_eq!(
                script.matches(&format!("printf '{CLEAR_SCREEN}'")).count(),
                1,
                "{shell}: the clear must appear once, as the last line, and nowhere else"
            );
            // And nothing before it writes to the screen at all, so the clear cannot be running early
            // in a script that has already done something visible.
            let body: String = script
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n");
            let without_the_clear = body.replace(&format!("printf '{CLEAR_SCREEN}'"), "");
            assert!(
                !without_the_clear.contains(r"\033["),
                "{shell}: only the clear may write to the screen: {without_the_clear}"
            );
        }
    }

    /// The injected line stays short, whatever the path looks like.
    ///
    /// The clear now covers the whole screen, so a line long enough to wrap is not left on screen — but
    /// it is still typed at a prompt, and a line that wraps is a line the line editor redraws in
    /// pieces. Short is one less thing between this and a terminal that opens cleanly.
    #[test]
    fn the_injected_line_is_one_short_row() {
        for shell in ["/bin/zsh", "/bin/bash"] {
            let (name, _) = shell_integration_script(Path::new(shell)).unwrap();
            let line = shell_integration_line(
                &Path::new("/home/a-rather-long-username")
                    .join(".marvis")
                    .join(name),
            );
            assert!(
                line.chars().count() < 60,
                "{shell}: {} chars, which can wrap behind a wide prompt: {line}",
                line.chars().count()
            );
            assert!(!line.contains('\n'));
        }
    }

    /// Where the script goes, and what is left behind when it is not wanted.
    ///
    /// It is written per terminal and never deleted, which is what makes turning the setting on
    /// part-way through a session work: there is no file to have existed at launch. It is written
    /// before the spawn, so the shell that reads it is never racing the write, and it is left alone
    /// when the setting is off so that a session opened with the setting on and then turned off does
    /// not have anything to strip out of a terminal that is already running.
    #[test]
    fn the_script_is_written_next_to_the_settings_and_only_when_it_is_wanted() {
        let directory = tempdir().unwrap();
        let zsh = Path::new("/bin/zsh");
        let line = install_shell_integration(zsh, Some(directory.path())).unwrap();
        assert_eq!(
            line,
            shell_integration_line(&directory.path().join("hook.zsh"))
        );
        assert!(directory.path().join("hook.zsh").is_file());
        // The two shells get their own files rather than one that dispatches on `$0`, and only the one
        // a terminal actually asked for is written.
        assert!(!directory.path().join("hook.bash").exists());
        let bash = Path::new("/bin/bash");
        if shell_integration_script(bash).is_some() {
            install_shell_integration(bash, Some(directory.path())).unwrap();
            assert!(directory.path().join("hook.bash").is_file());
            assert_ne!(
                fs::read_to_string(directory.path().join("hook.zsh")).unwrap(),
                fs::read_to_string(directory.path().join("hook.bash")).unwrap()
            );
        }
        // It is private to its owner, because it is code this app asks somebody's shell to run.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(directory.path().join("hook.zsh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "the script must not be readable by anyone else"
            );
        }
        // No folder to write into means no integration rather than a line pointing at nothing.
        assert_eq!(install_shell_integration(zsh, None), None);
        assert!(shell_integration_script(Path::new("/bin/sh")).is_none());
    }

    /// Every bash the machine has, oldest first.
    ///
    /// The script has to be right on all of them rather than on whichever one happens to come first,
    /// because `inherited_shell` spawns `$SHELL` and a person who has installed a bash has usually put
    /// it there on purpose. `/bin/bash` leads because on macOS that is the one most terminals get, and
    /// it is the oldest bash there is.
    fn bashes() -> Vec<String> {
        let mut found: Vec<String> = std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .map(String::from)
            .filter(|path| path.ends_with("/bash") && Path::new(path).is_file())
            .filter(|path| {
                Command::new(path)
                    .arg("--version")
                    .output()
                    .is_ok_and(|out| out.status.success())
            })
            .collect();
        found.dedup();
        if !found.iter().any(|path| path == "/bin/bash") {
            found.insert(0, "/bin/bash".into());
        }
        found
    }

    /// Whether this bash runs every element of a `PROMPT_COMMAND` array, or only the first.
    ///
    /// Asked of the shell rather than read off its version number, because it is the property the
    /// script branches on and therefore the one that has to hold. Measured false on bash 3.2, which is
    /// what `/bin/bash` is on macOS, so the script has to take the string branch there.
    fn bash_honours_prompt_command_arrays(bash: &str) -> bool {
        let run = "\
PROMPT_COMMAND=(probe_one probe_two)
probe_one() { printf '[ONE]'; }
probe_two() { printf '[TWO]'; }
";
        // `true` is there to draw one more prompt, and the answer is on it.
        bash_session_with(bash, run, &[b"true\n"], b"\x1b]133;D;0\x07").contains("[ONE][TWO]")
    }

    /// Opens a bash with a startup file of the test's own, runs the real install script against it, and
    /// hands back everything the shell printed.
    ///
    /// The user's prompt work and their DEBUG trap are installed by `--rcfile` rather than by the test
    /// writing to the PTY, because that is the order they exist in for real: a bashrc has run long
    /// before the line this app types, and a script that only survives because nothing was set up yet
    /// is not a script that leaves a shell alone.
    ///
    /// Every chunk is kept rather than drained at the end, because `wait_for_output` consumes the
    /// channel to find its marker and what came *before* that marker is the subject here.
    fn bash_session_with(bash: &str, rc: &str, commands: &[&[u8]], ends_with: &[u8]) -> String {
        let directory = tempdir().unwrap();
        let rcfile = directory.path().join("bashrc");
        fs::write(&rcfile, rc).unwrap();
        let line = script_in(directory.path(), bash);
        let backend = TerminalBackend::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        // Every chunk is kept rather than drained at the end, because `wait_for_output` consumes the
        // channel to find its marker and what came before that marker is the subject here.
        let collected: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = Arc::clone(&collected);
        let output: OutputSink = Box::new(move |bytes| {
            kept.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(bytes);
            sender.send(bytes.to_vec()).map_err(|e| e.to_string())
        });
        backend
            .spawn(
                "bash:existing".into(),
                SpawnOptions {
                    program: PathBuf::from(bash),
                    args: vec![
                        "--noprofile".into(),
                        "--rcfile".into(),
                        rcfile.to_string_lossy().into_owned(),
                        "-i".into(),
                    ],
                    cwd: std::env::current_dir().unwrap(),
                    cols: 80,
                    rows: 24,
                    startup_line: Some(line),
                },
                output,
            )
            .unwrap();
        wait_for_output(&receiver, b"\x1b]133;D;0\x07", Duration::from_secs(15));
        for command in commands {
            backend.write("bash:existing", command).unwrap();
        }
        // The last prompt drawn is where the effects of those commands are all in the stream, so the
        // wait is on that command's own status: `false` ends a prompt with `D;1`, not `D;0`.
        wait_for_output(&receiver, ends_with, Duration::from_secs(10));
        backend.close("bash:existing").unwrap();
        let stream = collected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        String::from_utf8_lossy(&stream).into_owned()
    }

    /// A bash script that installs itself has to leave the shell it lands in as it found it.
    ///
    /// Two things in it do reach into somebody's configuration, and both were doing it silently. The
    /// `PROMPT_COMMAND` assignment replaced a multi-element array a person had set up, taking every
    /// prompt command but the first with it; and `trap … DEBUG` replaced a DEBUG trap they had, with
    /// no error anywhere and no way back. A user on a bash 5.1+ who had two or three prompt commands
    /// in the array opened a terminal and found one of them gone.
    ///
    /// Both are asserted against a real bash with a real rc file rather than by comparing the script's
    /// text, because the subject is what the shell *does* with the text, and that is the class of bug
    /// that shipped a syntactically perfect `PROMPT_COMMAND="…"` in the first place.
    #[test]
    fn the_bash_script_keeps_a_prompt_and_a_debug_trap_that_were_already_there() {
        // A DEBUG trap of the user's own, which is the state every shell framework that manages a
        // prompt ends up in.
        const DEBUG_TRAP: &str = "trap 'printf \"<USER_DEBUG>\"' DEBUG\n";
        // The same three prompt commands as an array, which is what bash 5.1 and later use, and as a
        // string, which is what every bash before it uses and what a person who set one by hand has.
        const FUNCTIONS: &str = "\
user_prompt_one() { printf '[ONE:%s]' \"$?\"; }
user_prompt_two() { printf '[TWO:%s]' \"$?\"; }
user_prompt_three() { printf '[THREE:%s]' \"$?\"; }
";
        for bash in bashes() {
            let arrays_honoured = bash_honours_prompt_command_arrays(&bash);
            let rc = format!(
                "{FUNCTIONS}{DEBUG_TRAP}PROMPT_COMMAND=(user_prompt_one user_prompt_two user_prompt_three)\n"
            );
            let stream = bash_session_with(
                &bash,
                &rc,
                &[b"declare -p PROMPT_COMMAND\n", b"false\n"],
                b"\x1b]133;D;1\x07",
            );
            assert!(
                stream.contains("<USER_DEBUG>"),
                "{bash}: installing the integration silently dropped the user's DEBUG trap:\n{stream}"
            );
            // Still an array, with the user's entries where they were. A prompt framework that reads
            // or rewrites `PROMPT_COMMAND` is looking at exactly this.
            assert!(
                stream.contains("declare -a PROMPT_COMMAND"),
                "{bash}: PROMPT_COMMAND stopped being the array the user had:\n{stream}"
            );
            for entry in ["user_prompt_one", "user_prompt_two", "user_prompt_three"] {
                assert!(
                    stream.contains(entry),
                    "{bash}: PROMPT_COMMAND lost {entry}:\n{stream}"
                );
            }
            // Ours goes in as its own element rather than folded into element 0, which is the shape
            // that only matters where every element runs. On a bash that runs element 0 alone the
            // string form below is what keeps the user's commands, and asserting this there would be
            // asserting the wrong thing.
            if arrays_honoured {
                assert!(
                    stream.contains("[0]=\"__marvis_prompt_command\""),
                    "{bash}: our function was folded into element 0 rather than prepended to the array:\n{stream}"
                );
            }

            // The string form, which every bash runs in full — a bash before 5.1 runs only element 0 of
            // an array, so this is the one shape where all three can be watched running everywhere.
            let rc = format!(
                "{FUNCTIONS}{DEBUG_TRAP}PROMPT_COMMAND='user_prompt_one; user_prompt_two; user_prompt_three'\n"
            );
            let stream = bash_session_with(&bash, &rc, &[b"false\n"], b"\x1b]133;D;1\x07");
            for prompt in ["[ONE:", "[TWO:", "[THREE:"] {
                assert!(
                    stream.contains(prompt),
                    "{bash}: installing the integration dropped {prompt} from PROMPT_COMMAND:\n{stream}"
                );
            }
            // And the status is the command's rather than ours: `false` leaves 1, and a user's prompt
            // colouring is exactly the thing that reads it. Only the first of theirs can be holding
            // it — in a `;`-separated string each command resets `$?` for the next, which is what a
            // bare `PROMPT_COMMAND='a; b'` does with no integration installed either — so this is the
            // one assertion about `$?` that is about what this script does.
            assert!(
                stream.contains("[ONE:1]"),
                "{bash}: the first prompt command after ours saw $? = 0 rather than the command's status:\n{stream}"
            );
        }
    }

    /// Every terminal gets the same script, and it is not rewritten for each one.
    ///
    /// Opening a dozen terminals touching the disk once is the difference between a feature that is
    /// invisible and one that is merely cheap.
    #[test]
    fn the_script_is_written_once_and_reused() {
        let directory = tempdir().unwrap();
        let zsh = Path::new("/bin/zsh");
        let script = directory.path().join("hook.zsh");
        install_shell_integration(zsh, Some(directory.path())).unwrap();
        let written_at = fs::metadata(&script).unwrap().modified().unwrap();
        // A file whose contents no longer match must be replaced, or a change to the script would
        // never reach a terminal opened after it.
        fs::write(&script, "stale\n").unwrap();
        install_shell_integration(zsh, Some(directory.path())).unwrap();
        assert_ne!(
            fs::read_to_string(&script).unwrap(),
            "stale\n",
            "a script that no longer matches what this build writes must be replaced"
        );
        // And a second terminal with the script already right must leave it alone.
        let settled = fs::metadata(&script).unwrap().modified().unwrap();
        install_shell_integration(zsh, Some(directory.path())).unwrap();
        assert_eq!(fs::metadata(&script).unwrap().modified().unwrap(), settled);
        assert!(written_at <= settled);
    }

    /// Opens a real terminal with a real setting and reports whether the script reached it.
    ///
    /// Goes through `create_with_settings` rather than through the backend, because the setting is
    /// only read on the way in: what a terminal already has cannot be taken away by turning the
    /// setting off afterwards, which is what the field's own doc comment promises.
    ///
    /// Returns whether the app's folder ended up holding a script, which is the whole of what the
    /// setting decides: no file and no line, or a file and a line.
    fn opening_a_terminal_writes(script_dir: &Path, setting: Option<bool>) -> bool {
        let directory = tempdir().unwrap();
        let checkout = directory.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let database = Database::open(directory.path().join("workspace.sqlite3")).unwrap();
        let repo = plain_repo(&checkout);
        database.register_plain_repo(repo.clone()).unwrap();
        let backend = TerminalBackend::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        let output: OutputSink =
            Box::new(move |bytes| sender.send(bytes.to_vec()).map_err(|e| e.to_string()));

        let created = create_with_settings(
            &database,
            &backend,
            &repo.checkouts[0].id,
            TerminalOptions {
                cols: 80,
                rows: 24,
                prompt: None,
            },
            &setting,
            &Some(script_dir.to_path_buf()),
            output,
        )
        .unwrap();

        if setting == Some(false) {
            // Nothing is going to print a marker, so this waits on something the shell prints itself,
            // which is also what proves the session is live and not merely registered.
            backend
                .write(&created.session.id, b"printf 'MARVISPROBE\\n'\n")
                .unwrap();
            wait_for_output(&receiver, b"MARVISPROBE", Duration::from_secs(15));
        }
        assert!(backend.status(&created.session.id).is_ok());
        backend.close(&created.session.id).unwrap();
        script_dir.join("hook.zsh").is_file() || script_dir.join("hook.bash").is_file()
    }

    /// The setting decides what the next terminal gets, and nothing else moves.
    ///
    /// Off writes no script and sends no line, so a terminal behaves as it did before this existed: no
    /// script to source, so no OSC 133, so the frontend never has an exit code to turn a row red from.
    /// There is nothing to strip out of a terminal that is already open either — there was never
    /// anything in it to strip.
    ///
    /// `None` is "no settings file was reachable" and reads as on, because the feature ships on and a
    /// person whose file is broken already gets an error from the settings dialog. Reading it as off
    /// would quietly take the feature away from everybody whose file has not been written yet.
    #[test]
    fn the_setting_decides_whether_a_new_terminal_is_integrated() {
        if Command::new("/bin/zsh").arg("--version").output().is_err() {
            return;
        }
        assert!(opening_a_terminal_writes(
            tempdir().unwrap().path(),
            Some(true)
        ));
        let off = tempdir().unwrap();
        assert!(!opening_a_terminal_writes(off.path(), Some(false)));
        assert!(
            !off.path().join("hook.zsh").exists(),
            "nothing may be written when the setting is off"
        );
        assert!(opening_a_terminal_writes(tempdir().unwrap().path(), None));
    }

    /// Writes a capture of a real session's output for `terminal-shell-integration-render.test.ts` to
    /// replay through a real terminal.
    ///
    /// This is the only test here that can see the erase, because the erase's bytes are in the stream
    /// whether or not they are honoured: a screen is what tells a line that was drawn from one that
    /// was drawn and then taken back off. The frontend test asserts on the rendered screen, and this
    /// is where the bytes come from.
    ///
    /// `#[ignore]`d because it writes into the source tree and the render test reads a committed
    /// capture. Regenerate with:
    ///
    /// ```text
    /// MARVIS_CAPTURE_FIXTURES=src/lib/__fixtures__ cargo test --manifest-path src-tauri/Cargo.toml \
    ///   captures_a_real_session_for_the_render_test -- --ignored
    /// ```
    #[test]
    #[ignore = "regenerates committed fixtures; see MARVIS_CAPTURE_FIXTURES"]
    fn captures_a_real_session_for_the_render_test() {
        let Some(directory) = std::env::var_os("MARVIS_CAPTURE_FIXTURES") else {
            panic!("set MARVIS_CAPTURE_FIXTURES to the directory to write captures into");
        };
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        for (name, shell, args) in [
            ("zsh-login", "/bin/zsh", &["-l", "-i"][..]),
            ("zsh-bare", "/bin/zsh", &["-f", "-i"][..]),
            ("bash-login", "/bin/bash", &["-l", "-i"][..]),
        ] {
            if Command::new(shell).arg("--version").output().is_err() {
                continue;
            }
            let stream = capture_a_session(shell, args, tempdir().unwrap().path());
            fs::write(directory.join(format!("{name}.bin")), &stream).unwrap();
            println!("captured {name}: {} bytes", stream.len());
        }
        // Two controls, both of which the render test needs to be able to fail. Without the first, "the
        // terminal opens pristine" could be an instrument that never saw the residue. Without the
        // second, waiting for the shell to be ready could look like it makes no difference.
        let stream = capture_a_session_without_the_clear();
        fs::write(directory.join("control-no-clear.bin"), &stream).unwrap();
        println!("captured control-no-clear: {} bytes", stream.len());

        // Held, not dropped at the end of the statement: the directory has to still be there when the
        // shell reads the script out of it.
        let script_dir = tempdir().unwrap();
        let line = script_in(script_dir.path(), "/bin/zsh");
        let stream = capture_a_stream("/bin/zsh", &["-l", "-i"], Some(line), false);
        fs::write(directory.join("control-no-settle.bin"), &stream).unwrap();
        println!("captured control-no-settle: {} bytes", stream.len());
    }

    /// A real session's output: the injected line, the script's own markers, and a failing command.
    fn capture_a_session(shell: &str, args: &[&str], script_dir: &Path) -> Vec<u8> {
        let line = script_in(script_dir, shell);
        capture_a_stream(shell, args, Some(line), true)
    }

    /// The same, with a script that registers nothing and never clears, so the setup leaves its mark.
    fn capture_a_session_without_the_clear() -> Vec<u8> {
        let directory = tempdir().unwrap();
        let script = directory.path().join("hook.zsh");
        fs::write(
            &script,
            "# Captured without the clear, as the control for the render test.\n\
             marvis_pc() { print -Pn \"\\e]133;D;$?\\a\"; }\n\
             precmd_functions+=(marvis_pc)\n",
        )
        .unwrap();
        capture_a_stream(
            "/bin/zsh",
            &["-l", "-i"],
            Some(shell_integration_line(&script)),
            true,
        )
    }

    /// Drains the receiver until it has been quiet for `quiet_for`, which is the observable that means
    /// the shell has stopped talking and its line editor has taken over.
    fn wait_for_quiet(receiver: &std::sync::mpsc::Receiver<Vec<u8>>, quiet_for: Duration) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match receiver.recv_timeout(quiet_for) {
                Ok(_) => continue,
                Err(_) => return,
            }
        }
    }

    /// Spawns a shell, waits for it to install `startup_line`, then fails a command and returns
    /// everything the session printed.
    fn capture_a_stream(
        shell: &str,
        args: &[&str],
        startup_line: Option<String>,
        settle: bool,
    ) -> Vec<u8> {
        let backend = TerminalBackend::default();
        let captured: std::sync::Arc<std::sync::Mutex<Vec<u8>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (sender, receiver) = std::sync::mpsc::channel();
        let kept = std::sync::Arc::clone(&captured);
        let output: OutputSink = Box::new(move |bytes| {
            kept.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(bytes);
            sender.send(bytes.to_vec()).map_err(|e| e.to_string())
        });
        let session = "session:capture".to_string();
        // `settle` false is the line written the moment the shell exists, while it is still loading its
        // startup files. Production waits; this is what it is waiting for.
        backend
            .spawn(
                session.clone(),
                SpawnOptions {
                    program: Path::new(shell).to_path_buf(),
                    args: args.iter().map(|arg| (*arg).to_string()).collect(),
                    cwd: std::env::current_dir().unwrap(),
                    cols: 80,
                    rows: 24,
                    startup_line: if settle { startup_line.clone() } else { None },
                },
                output,
            )
            .unwrap();
        if !settle {
            backend
                .write(
                    &session,
                    format!("{}\n", startup_line.unwrap_or_default()).as_bytes(),
                )
                .unwrap();
        }
        wait_for_output(&receiver, b"\x1b]133;D;0\x07", Duration::from_secs(20));
        // The prompt is still being drawn when that marker arrives, and typing into a shell that has
        // not finished drawing one is the same race the startup settle exists for — it would put an
        // extra render of `false` into the capture and make it a worse picture of a real session than
        // the app produces. So: wait for the quiet that means the line editor is armed.
        wait_for_quiet(&receiver, Duration::from_millis(400));
        backend.write(&session, b"false\n").unwrap();
        wait_for_output(&receiver, b"\x1b]133;D;1\x07", Duration::from_secs(15));
        backend.close(&session).unwrap();
        // The sink is gone by now, so the Arc is the only thing still holding the bytes.
        let stream = captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        stream
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
