use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use terminal_backend::{OutputSink, SpawnOptions, TerminalBackend, TerminalId};

fn spawn(
    backend: &TerminalBackend,
    program: &str,
    args: &[String],
    on_output: OutputSink,
) -> TerminalId {
    backend
        .spawn(
            SpawnOptions {
                program: PathBuf::from(program),
                args: args.to_vec(),
                cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .to_path_buf(),
                cols: 80,
                rows: 24,
            },
            on_output,
        )
        .unwrap()
}

fn channel_sink() -> (OutputSink, mpsc::Receiver<Vec<u8>>) {
    let (sender, receiver) = mpsc::channel();
    (
        Box::new(move |bytes| {
            sender
                .send(bytes.to_vec())
                .map_err(|error| error.to_string())
        }),
        receiver,
    )
}

fn read_exactly(receiver: &mpsc::Receiver<Vec<u8>>, size: usize, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::with_capacity(size);
    while bytes.len() < size {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let chunk = receiver
            .recv_timeout(remaining)
            .unwrap_or_else(|error| panic!("received {} of {size} bytes: {error}", bytes.len()));
        bytes.extend_from_slice(&chunk);
    }
    bytes
}

fn percentile(sorted: &[u128], percent: usize) -> u128 {
    sorted[((sorted.len() - 1) * percent).div_ceil(100)]
}

#[test]
fn large_cat_preserves_every_byte() {
    let backend = TerminalBackend::default();
    let fixture = std::env::temp_dir().join(format!(
        "marvis-pty-cat-{}-{}.bin",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let expected: Vec<u8> = (0..16 * 1024 * 1024)
        .map(|index| 33 + (index % 94) as u8)
        .collect();
    fs::write(&fixture, &expected).unwrap();
    let (sink, output) = channel_sink();
    let id = spawn(
        &backend,
        "cat",
        &[fixture.to_string_lossy().into_owned()],
        sink,
    );

    let start = Instant::now();
    let actual = read_exactly(&output, expected.len(), Duration::from_secs(30));
    let elapsed = start.elapsed();
    assert_eq!(actual, expected);
    let metrics = backend.metrics(id).unwrap();
    assert_eq!(metrics.bytes_read, expected.len() as u64);
    assert_eq!(metrics.bytes_sent, expected.len() as u64);
    assert_eq!(metrics.dropped_bytes, 0);
    println!(
        "large cat: {} bytes, {:.1} MiB/s, dropped={} bytes",
        expected.len(),
        expected.len() as f64 / elapsed.as_secs_f64() / 1_048_576.0,
        metrics.dropped_bytes
    );

    backend.terminate(id).unwrap();
    fs::remove_file(fixture).unwrap();
}

#[test]
fn input_echo_and_rapid_resize_are_measured() {
    let backend = TerminalBackend::default();
    let (sink, output) = channel_sink();
    let id = spawn(&backend, "cat", &[], sink);
    let mut latencies = Vec::new();
    let mut received = Vec::new();

    for index in 0..40 {
        let marker = format!("latency_probe_{index:02x}");
        let start = Instant::now();
        backend.write(id, marker.as_bytes()).unwrap();
        while !received
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
        {
            received.extend_from_slice(
                &output
                    .recv_timeout(Duration::from_secs(3))
                    .expect("PTY echo timeout"),
            );
        }
        latencies.push(start.elapsed().as_micros());
    }

    let mut resize_times = Vec::new();
    for index in 0..200 {
        let cols = 60 + (index % 100) as u16;
        let rows = 20 + (index % 50) as u16;
        let start = Instant::now();
        let actual = backend.resize(id, cols, rows).unwrap();
        resize_times.push(start.elapsed().as_micros());
        assert_eq!(actual, (cols, rows));
    }
    latencies.sort_unstable();
    resize_times.sort_unstable();
    println!(
        "PTY echo latency: median={}µs p95={}µs; resize: 200 operations, median={}µs p95={}µs",
        percentile(&latencies, 50),
        percentile(&latencies, 95),
        percentile(&resize_times, 50),
        percentile(&resize_times, 95),
    );
    backend.terminate(id).unwrap();
}

#[test]
fn five_concurrent_ptys_are_independent() {
    let backend = TerminalBackend::default();
    let mut terminals = Vec::new();
    for _ in 0..5 {
        let (sink, output) = channel_sink();
        let id = spawn(&backend, "cat", &[], sink);
        terminals.push((id, output));
    }

    let start = Instant::now();
    for (index, (id, _)) in terminals.iter().enumerate() {
        backend
            .write(*id, format!("terminal-{index}\n").as_bytes())
            .unwrap();
    }
    for (index, (_, output)) in terminals.iter().enumerate() {
        let mut received = Vec::new();
        let expected = format!("terminal-{index}");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !received
            .windows(expected.len())
            .any(|window| window == expected.as_bytes())
        {
            let chunk = output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("terminal output timeout");
            received.extend_from_slice(&chunk);
        }
    }
    println!(
        "five independent PTYs: echo available in {:.2}ms",
        start.elapsed().as_secs_f64() * 1000.0
    );

    for (id, _) in terminals {
        backend.terminate(id).unwrap();
    }
}

#[test]
fn ripgrep_runs_through_the_pty() {
    let root = std::env::temp_dir().join(format!("marvis-pty-rg-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let fixture = root.join("file with spaces.txt");
    fs::write(&fixture, "first line\nneedle on second line\nlast line\n").unwrap();
    let (sink, output) = channel_sink();
    let backend = TerminalBackend::default();
    let id = spawn(
        &backend,
        "rg",
        &[
            "--color=never".into(),
            "needle".into(),
            fixture.to_string_lossy().into_owned(),
        ],
        sink,
    );
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match output.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("rg output timeout"),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if bytes
            .windows(b"needle on second line".len())
            .any(|window| window == b"needle on second line")
        {
            break;
        }
    }
    assert!(bytes
        .windows(b"needle on second line".len())
        .any(|window| window == b"needle on second line"));
    println!("rg: matched fixture path containing spaces through portable-pty");

    backend.terminate(id).unwrap();
    drop(output);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cargo_build_and_pnpm_install_run_through_the_pty() {
    let backend = TerminalBackend::default();
    let commands = [
        (
            "cargo",
            vec![
                "build".into(),
                "--manifest-path".into(),
                "src-tauri/Cargo.toml".into(),
                "--locked".into(),
            ],
            "Finished",
        ),
        (
            "pnpm",
            vec!["install".into(), "--frozen-lockfile".into()],
            "Done in",
        ),
    ];

    for (program, args, completion) in commands {
        let (sink, output) = channel_sink();
        let id = spawn(&backend, program, &args, sink);
        let started = Instant::now();
        let deadline = started + Duration::from_secs(180);
        let mut received = Vec::new();
        while !backend.metrics(id).unwrap().reader_closed {
            match output.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => received.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    while !backend.metrics(id).unwrap().reader_closed {
                        thread::yield_now();
                    }
                }
                Err(error) => panic!("{program} did not finish through PTY: {error}"),
            }
            assert!(Instant::now() < deadline, "{program} timed out in PTY");
        }
        while let Ok(chunk) = output.try_recv() {
            received.extend_from_slice(&chunk);
        }
        let log = String::from_utf8_lossy(&received);
        assert!(
            log.contains(completion),
            "{program} output did not contain {completion:?}: {log}"
        );
        println!(
            "{program}: {} PTY bytes in {:.2}s",
            received.len(),
            started.elapsed().as_secs_f64()
        );
        backend.terminate(id).unwrap();
    }
}

#[test]
fn nvim_and_htop_start_in_a_resizable_pty() {
    let backend = TerminalBackend::default();
    for (program, args) in [("nvim", vec!["--clean".into()]), ("htop", vec![])] {
        let (sink, output) = channel_sink();
        let id = spawn(&backend, program, &args, sink);
        let first_output = output.recv_timeout(Duration::from_secs(5));
        assert!(first_output.is_ok(), "{program} produced no PTY output");
        let (cols, rows) = backend.resize(id, 91, 31).unwrap();
        assert_eq!((cols, rows), (91, 31));
        println!("{program}: started and received PTY resize 91x31");
        backend.terminate(id).unwrap();
    }
}

#[test]
#[ignore = "runs yes for the plan's full 30 seconds"]
fn yes_streams_for_thirty_seconds() {
    let backend = TerminalBackend::default();
    let total = Arc::new(AtomicU64::new(0));
    let sink_total = Arc::clone(&total);
    let id = spawn(
        &backend,
        "yes",
        &[],
        Box::new(move |bytes| {
            sink_total.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            Ok(())
        }),
    );
    let pid = backend.process_id(id).unwrap().unwrap();
    let started = Instant::now();
    let mut samples = Vec::new();
    while started.elapsed() < Duration::from_secs(30) {
        thread::sleep(Duration::from_secs(5).min(Duration::from_secs(30) - started.elapsed()));
        let usage = Command::new("ps")
            .args(["-o", "%cpu=,rss=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        samples.push(String::from_utf8_lossy(&usage.stdout).trim().to_string());
    }

    let measured = started.elapsed();
    backend.write(id, &[3]).unwrap();
    let drain_deadline = Instant::now() + Duration::from_secs(5);
    while !backend.metrics(id).unwrap().reader_closed && Instant::now() < drain_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let metrics = backend.metrics(id).unwrap();
    let bytes = total.load(Ordering::Relaxed);
    println!(
        "yes: {:.3}s, {} bytes, {:.2} MiB/s, backend dropped={} bytes, child ps samples (%cpu,rss-KiB)={samples:?}",
        measured.as_secs_f64(),
        bytes,
        bytes as f64 / measured.as_secs_f64() / 1_048_576.0,
        metrics.dropped_bytes
    );
    assert!(
        metrics.reader_closed,
        "PTY output did not drain after Ctrl-C"
    );
    assert_eq!(metrics.bytes_read, bytes);
    assert_eq!(metrics.bytes_sent, bytes);
    assert_eq!(metrics.dropped_bytes, 0);
    backend.terminate(id).unwrap();
}
