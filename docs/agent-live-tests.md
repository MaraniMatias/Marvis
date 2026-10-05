# Agent integration tests

Marvis is a client of the OpenCode service the user starts; it never starts or stops one. The
Rust tests separate local contract and lifecycle checks from tests that reach a real OpenCode
service or send prompts to a provider. The six live checks remain `#[ignore]`; default tests and
CI do not run them.

## Per-terminal identity

OpenCode 2.0.22 exposes no way to attribute a session to the TUI showing it. There is no route
that lists connected clients, `x-opencode-client` is only ever sent _out_ to model providers, and
`/api/session/active` answers for the whole service rather than for one connection. `session.view`
sets one global `viewed` timestamp per session, so with two TUIs sharing a checkout the last one to
look wins.

What a terminal row can therefore be drawn from is the directory that terminal runs in, which is
what `useTerminalAgentRows` and the sidebar's `agentRows` are scoped by. A row names its own
worktree's agent and busy state and never claims a session id for itself. Inferring one from the
last-viewed session, the newest session, or the process name would misattribute as soon as two
TUIs share a checkout, so nothing here does that. See the note at the top of
[`services/agent.rs`](../src-tauri/src/services/agent.rs) for the same limit on the native side.

## Hermetic checks

Run from the repository root. These use local fixtures, temporary directories, and loopback
mock servers; they do not invoke OpenCode or a model provider. Ignored tests are excluded by
default.

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml services::agent::tests
cargo test --locked --manifest-path src-tauri/Cargo.toml services::review_round::tests
cargo test --locked --manifest-path src-tauri/Cargo.toml services::worktree::tests::stale_agent_request_cannot_cross_close_and_reopen_but_new_generation_can
cargo test --locked --manifest-path src-tauri/Cargo.toml services::worktree::tests::locating_a_moved_worktree_invalidates_old_agent_path_and_keeps_checkout_history
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

## Contract status

| Contract                                                                                         | Hermetic verified                                                                                   | Live not executed here                                                              |
| ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Loopback Basic auth, agent/session catalogs, foreign-session filtering, prompt body and response | `owned_loopback_api_auth_scopes_catalog_and_prompt_contract_to_checkout`                            | `the_agent_catalog_is_readable_from_a_real_server`, `bridge_talks_to_a_real_server` |
| Session ownership, stale event/request epochs, idle SSE parsing and reader stop                  | Agent service tests; the two targeted worktree generation tests above                               | `bridge_talks_to_a_real_server`, `a_turn_is_observable_through_the_idle_time`       |
| Review marker persisted in the prompt; retry decision from a transcript response                 | Review-round tests, including `reconciliation_confirms_a_present_marker_and_requeues_an_absent_one` | `a_round_marker_reaches_the_real_session`, `a_queued_round_goes_out_when_flushed`   |
| Model-driven review editing across two checkouts                                                 | Not covered hermetically                                                                            | `two_checkouts_run_the_review_loop`                                                 |
| JSON requests scoped by directory; SSE carries the checkout header                               | `every_request_carries_the_directory_as_a_header_and_as_a_query_parameter`, SSE response checks     | —                                                                                   |
| Running state read from the service, not inferred from an idle time                              | `owned_loopback_api_auth_scopes_catalog_and_prompt_contract_to_checkout`                            | —                                                                                   |
| Two terminals in different worktrees diverge; a terminal only reads its own worktree             | `useTerminalAgentRows` and `Sidebar workdir rows` vitest suites                                     | —                                                                                   |

The directory scoping, `/api/session/active` running answer, and the absence of any per-client
identity were checked against a running OpenCode **2.0.22** service with read-only requests
(`/openapi.json`, `/api/info`, `/api/session`, `/api/agent`, `/api/session/active`, `/api/event`).
No service was started or stopped for this, and no prompt was sent. The prompt `{ "text": ... }`
body is a source-level expectation that no live test here has confirmed.

## Opt-in live checks

Run only with explicit manual consent. They need the OpenCode CLI on `PATH`; the prompt tests
also need a configured provider and may incur provider charges. `two_checkouts_run_the_review_loop`
sends a review prompt and lets the model edit two files. Only disposable directories under the
system temp directory are safe inputs: never point either variable at a repository, worktree,
or other valuable data. The test sessions/configuration still use the local OpenCode installation.

Run one test at a time and allocate fresh directories for each invocation. The catalog-only test
creates a server and session but does not send a model prompt.

```sh
MARVIS_AGENT_BRIDGE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/marvis-agent.XXXXXX")" \
  cargo test --locked --manifest-path src-tauri/Cargo.toml a_round_marker_reaches_the_real_session -- --ignored --nocapture
```

The ignored test filters are:

| Test                                               | Provider prompt / edits                                     |
| -------------------------------------------------- | ----------------------------------------------------------- |
| `a_round_marker_reaches_the_real_session`          | Sends a prompt                                              |
| `a_turn_is_observable_through_the_idle_time`       | Sends a prompt                                              |
| `the_agent_catalog_is_readable_from_a_real_server` | No prompt; creates a session                                |
| `bridge_talks_to_a_real_server`                    | Sends a prompt                                              |
| `two_checkouts_run_the_review_loop`                | Sends a prompt; edits two files in the first temp directory |
| `a_queued_round_goes_out_when_flushed`             | Sends a prompt                                              |

For each listed test, replace the filter in the example and create a new temp directory.
`bridge_talks_to_a_real_server` and `two_checkouts_run_the_review_loop` also require a second
dedicated directory; use this two-directory pattern for either filter:

```sh
MARVIS_AGENT_BRIDGE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/marvis-agent-first.XXXXXX")" \
MARVIS_AGENT_BRIDGE_OTHER_DIR="$(mktemp -d "${TMPDIR:-/tmp}/marvis-agent-second.XXXXXX")" \
  cargo test --locked --manifest-path src-tauri/Cargo.toml two_checkouts_run_the_review_loop -- --ignored --nocapture
```

These commands are intentionally opt-in and must not be added to automatic CI.
