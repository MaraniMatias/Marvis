# Local persistence load benchmark

Run from the repository root:

```sh
node scripts/benchmark-persistence-load.mjs
```

The runner invokes only the ignored Rust test `persistence::tests::bounded_database_load_metrics` with Cargo offline. It does not select the six ignored live-provider tests. No app, provider, telemetry, network service, or cloud infrastructure is started.

The existing `git-spike bench` measures sequential Git `status --porcelain=v2` calls over temporary repositories (defaults: 100/5,000/50,000 tracked files, five repeats); it does not exercise Marvis or `Database`. Keep using it for Git-only cost, not as evidence for these persistence load paths.

The test creates an isolated temporary SQLite database and 12 temporary plain-checkout directories through `Database::register_plain_repo`. It measures the public `Database::load_workspace` and `Database::load_registered_checkout` paths, 25 calls each; setup and correctness assertions are outside the timed interval. These loaders verify that checkout directories exist; they do not scan checkout file contents. It then removes one fixture directory and verifies both loaders report it missing, and that an unknown checkout returns `None`. `error_count` counts `Err` results and is asserted to remain zero; wrong or missing data fails the test rather than being counted as success.

The test reports p50/p95 nanoseconds and error counts without printing fixture paths. The fixed dataset and repetition counts keep the run bounded. The runner enforces a five-minute process timeout and removes its dedicated temporary root after normal completion or timeout; `tempfile` also cleans the fixture on ordinary test exit.

This is a local test-fixture measurement, not production-scale capacity evidence or telemetry. Results are hardware/build-local; there are no latency thresholds in CI. It does not characterize Git snapshot cost, file-tree scanning, or cloud/production workloads. Keep the fixture/repetition limits fixed unless a specific measurement question justifies changing them.
