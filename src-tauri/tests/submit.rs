use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use talminal_canvas_lib::profiles;
use talminal_canvas_lib::prompt_readiness::PromptReadiness;
use talminal_canvas_lib::pty::{PtyHost, PtySpawn};
use talminal_canvas_lib::{registry, submit};

mod common;

#[test]
fn submit_prompt_delegates_to_submit_prompt_as_with_human_source() {
    let old = submit::submit_prompt("card-does-not-exist".into(), "x".into()).unwrap_err();
    let new = submit::submit_prompt_as(
        "card-does-not-exist".into(),
        "x".into(),
        submit::WriteSource::Human,
    )
    .unwrap_err();
    assert_eq!(old, new);
}

#[test]
fn write_source_labels_are_the_frozen_control_contract() {
    assert_eq!(submit::WriteSource::Human.as_str(), "human");
    assert_eq!(submit::WriteSource::Agent.as_str(), "agent");
}

fn cmd_exe() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!(r"{root}\System32\cmd.exe")
}

fn wait_for(
    bytes: &Arc<Mutex<Vec<(Instant, u8)>>>,
    needle: &[u8],
    occurrences: usize,
    timeout: Duration,
) -> Vec<Instant> {
    let started = Instant::now();
    loop {
        let matches = occurrence_times(&bytes.lock().unwrap(), needle);
        if matches.len() >= occurrences {
            return matches;
        }
        assert!(
            started.elapsed() < timeout,
            "did not observe {occurrences} occurrences of {:?}; raw output: {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(
                &bytes
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(_, byte)| *byte)
                    .collect::<Vec<_>>()
            )
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn occurrence_times(bytes: &[(Instant, u8)], needle: &[u8]) -> Vec<Instant> {
    if needle.is_empty() || bytes.len() < needle.len() {
        return Vec::new();
    }
    bytes
        .windows(needle.len())
        .filter_map(|window| {
            window
                .iter()
                .map(|(_, byte)| *byte)
                .eq(needle.iter().copied())
                .then_some(window[0].0)
        })
        .collect()
}

fn wait_for_prompt(bytes: &Arc<Mutex<Vec<(Instant, u8)>>>) {
    let started = Instant::now();
    loop {
        if bytes.lock().unwrap().iter().any(|(_, byte)| *byte == b'>') {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "cmd prompt not observed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn observe_initial_input(readiness: &PromptReadiness) {
    readiness.observe(
        u64::MAX,
        b"\x1B[?1049h>\xC2\xA0Try refactor this file\x1B[27;3H\x1B[?25h",
    );
}

fn observe_input_redraw(readiness: &PromptReadiness, text: &str) {
    let mut output = text.as_bytes().to_vec();
    output.extend_from_slice(b"\x1B[27;93H\x1B[?25h");
    readiness.observe(u64::MAX, &output);
}

type GatedProfileCard = (
    registry::CardInfo,
    Arc<Mutex<Vec<(Instant, u8)>>>,
    Arc<PtyHost>,
    Arc<PromptReadiness>,
);

fn gated_profile_card() -> (tempfile::TempDir, GatedProfileCard) {
    let dir = tempfile::tempdir().unwrap();
    // `None` makes this a real profile card (the production readiness path),
    // while the injected cmd PTY keeps the integration tests deterministic
    // and offline.
    let info = registry::create_card(dir.path().display().to_string(), "claude".to_string(), None)
        .expect("create profile card");
    let handle = registry::card_handle(&info.name).expect("card handle");
    let observed: Arc<Mutex<Vec<(Instant, u8)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    let host = Arc::new(
        PtyHost::spawn(
            PtySpawn {
                cwd: PathBuf::from(&info.cwd),
                command: vec![cmd_exe()],
                cols: 120,
                rows: 35,
                env_deny_prefixes: vec![],
                env_deny_exact: vec![],
                extra_env: vec![],
            },
            move |chunk| {
                let now = Instant::now();
                sink.lock()
                    .unwrap()
                    .extend(chunk.iter().copied().map(|byte| (now, byte)));
            },
        )
        .expect("spawn deterministic profile PTY"),
    );
    let readiness = Arc::new(PromptReadiness::new(&profiles::CLAUDE_READINESS));
    {
        let mut card = handle.lock().unwrap();
        let terminal = card.terminal_mut().expect("terminal card");
        terminal.pty = Some(Arc::clone(&host));
        terminal.submit_readiness = Some(Arc::clone(&readiness));
    }
    wait_for_prompt(&observed);
    (dir, (info, observed, host, readiness))
}

#[test]
fn submit_prompt_writes_text_then_carriage_return_after_profile_gap() {
    // Sandkasse FOERST: under --features supervision skriver submit-vejen et
    // pause-signal i talminal_base()/signals, og uden serial() er den rod
    // ejerens levende installation.
    let _g = common::serial();
    let dir = tempfile::tempdir().unwrap();
    let info = registry::create_card(
        dir.path().display().to_string(),
        "claude".to_string(),
        Some(cmd_exe()),
    )
    .expect("create echo card");
    let handle = registry::card_handle(&info.name).expect("card handle");
    let observed: Arc<Mutex<Vec<(Instant, u8)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    let host = Arc::new(
        PtyHost::spawn(
            PtySpawn {
                cwd: PathBuf::from(&info.cwd),
                command: vec![cmd_exe()],
                cols: 120,
                rows: 35,
                env_deny_prefixes: vec![],
                env_deny_exact: vec![],
                extra_env: vec![],
            },
            move |chunk| {
                let now = Instant::now();
                sink.lock()
                    .unwrap()
                    .extend(chunk.iter().copied().map(|byte| (now, byte)));
            },
        )
        .expect("spawn cmd echo card"),
    );
    handle
        .lock()
        .unwrap()
        .terminal_mut()
        .expect("terminal card")
        .pty = Some(Arc::clone(&host));
    wait_for_prompt(&observed);

    let marker = b"SUBMIT-GAP-SENTINEL";
    submit::submit_prompt(info.name.clone(), "echo SUBMIT-GAP-SENTINEL".to_string())
        .expect("submit_prompt");

    let occurrences = wait_for(&observed, marker, 2, Duration::from_secs(10));
    let gap = occurrences[1].duration_since(occurrences[0]);
    let required = Duration::from_millis(
        profiles::profile("claude")
            .expect("claude profile")
            .submit_gap_ms,
    );
    // Gabet maales READER-side: begge tidsstempler baerer OS-timer-/scheduling-
    // jitter, saa en writer der korrekt venter hele gabet kan maales ~3 ms for
    // kort (observeret 347,2 ms mod 350 ms). Marginen daekker KUN maalestoej —
    // writerens faktiske ventetid er uaendret.
    let jitter_margin = Duration::from_millis(15);
    assert!(
        gap + jitter_margin >= required,
        "second write reached the reader after {gap:?}, expected at least {required:?} minus {jitter_margin:?} measurement jitter"
    );

    host.write(b"exit\r").expect("exit write");
    registry::close_card(info.name).expect("close cleanup");
}

#[test]
fn profile_submit_does_not_write_until_live_prompt_is_observed() {
    // Sandkasse FOERST: under --features supervision skriver submit-vejen et
    // pause-signal i talminal_base()/signals, og uden serial() er den rod
    // ejerens levende installation.
    let _g = common::serial();
    let (_dir, (info, observed, _host, readiness)) = gated_profile_card();

    let marker = b"P5-READINESS-SENTINEL";
    let prompt = "echo P5-READINESS-SENTINEL".to_string();
    let name = info.name.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let submitted_prompt = prompt.clone();
    let submitter = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx
            .send(submit::submit_prompt(name, submitted_prompt))
            .unwrap();
    });
    started_rx.recv().unwrap();
    assert!(
        done_rx.recv_timeout(Duration::from_millis(150)).is_err(),
        "submit returned before readiness"
    );
    assert!(
        occurrence_times(&observed.lock().unwrap(), marker).is_empty(),
        "prompt text reached the PTY before readiness"
    );

    observe_initial_input(&readiness);
    wait_for(&observed, marker, 1, Duration::from_secs(1));
    assert!(
        done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "submit returned before the post-text input redraw"
    );
    observe_input_redraw(&readiness, &prompt);
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("submit completed after post-text redraw")
        .expect("submit result");
    wait_for(&observed, marker, 2, Duration::from_secs(5));
    submitter.join().unwrap();

    registry::close_card(info.name).expect("close cleanup");
}

#[test]
fn concurrent_profile_submits_are_serialized_per_pty_run() {
    // Sandkasse FOERST: under --features supervision skriver submit-vejen et
    // pause-signal i talminal_base()/signals, og uden serial() er den rod
    // ejerens levende installation.
    let _g = common::serial();
    let (_dir, (info, observed, _host, readiness)) = gated_profile_card();
    observe_initial_input(&readiness);
    let first = "echo P5-SERIAL-FIRST".to_string();
    let second = "echo P5-SERIAL-SECOND".to_string();

    let first_name = info.name.clone();
    let first_prompt = first.clone();
    let (first_tx, first_rx) = std::sync::mpsc::channel();
    let first_thread = std::thread::spawn(move || {
        first_tx
            .send(submit::submit_prompt(first_name, first_prompt))
            .unwrap();
    });
    wait_for(&observed, b"P5-SERIAL-FIRST", 1, Duration::from_secs(1));

    let second_name = info.name.clone();
    let second_prompt = second.clone();
    let (second_tx, second_rx) = std::sync::mpsc::channel();
    let second_thread = std::thread::spawn(move || {
        second_tx
            .send(submit::submit_prompt(second_name, second_prompt))
            .unwrap();
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        occurrence_times(&observed.lock().unwrap(), b"P5-SERIAL-SECOND").is_empty(),
        "second prompt reached the PTY while the first owned its input widget"
    );
    assert!(second_rx.try_recv().is_err());

    observe_input_redraw(&readiness, &first);
    first_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first submit completed")
        .expect("first submit result");
    wait_for(&observed, b"P5-SERIAL-FIRST", 2, Duration::from_secs(2));
    assert!(
        second_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "second submit reused the first prompt's pre-Enter redraw"
    );
    observe_initial_input(&readiness);
    wait_for(&observed, b"P5-SERIAL-SECOND", 1, Duration::from_secs(2));
    observe_input_redraw(&readiness, &second);
    second_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("second submit completed")
        .expect("second submit result");
    wait_for(&observed, b"P5-SERIAL-SECOND", 2, Duration::from_secs(2));

    first_thread.join().unwrap();
    second_thread.join().unwrap();
    registry::close_card(info.name).expect("close cleanup");
}

#[test]
fn cancelling_pending_readiness_returns_without_writing() {
    // Sandkasse FOERST: under --features supervision skriver submit-vejen et
    // pause-signal i talminal_base()/signals, og uden serial() er den rod
    // ejerens levende installation.
    let _g = common::serial();
    let (_dir, (info, observed, _host, readiness)) = gated_profile_card();
    let marker = b"P5-CANCEL-PENDING";
    let name = info.name.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let submitter = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx
            .send(submit::submit_prompt(
                name,
                "echo P5-CANCEL-PENDING".to_string(),
            ))
            .unwrap();
    });
    started_rx.recv().unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    readiness.cancel();
    let error = done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("cancel wakes submit")
        .expect_err("cancelled submit fails closed");
    assert!(error.contains("readiness cancelled"), "{error}");
    assert!(occurrence_times(&observed.lock().unwrap(), marker).is_empty());
    submitter.join().unwrap();
    registry::close_card(info.name).expect("close cleanup");
}

#[test]
fn cancelling_during_submit_gap_prevents_carriage_return() {
    // Sandkasse FOERST: under --features supervision skriver submit-vejen et
    // pause-signal i talminal_base()/signals, og uden serial() er den rod
    // ejerens levende installation.
    let _g = common::serial();
    let (_dir, (info, observed, _host, readiness)) = gated_profile_card();
    observe_initial_input(&readiness);
    let marker = b"P5-CANCEL-GAP";
    let prompt = "echo P5-CANCEL-GAP".to_string();
    let name = info.name.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let submitted_prompt = prompt.clone();
    let submitter = std::thread::spawn(move || {
        done_tx
            .send(submit::submit_prompt(name, submitted_prompt))
            .unwrap();
    });
    wait_for(&observed, marker, 1, Duration::from_secs(1));
    observe_input_redraw(&readiness, &prompt);
    readiness.cancel();
    let error = done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("gap cancellation wakes submit")
        .expect_err("cancelled gap fails closed");
    assert!(error.contains("readiness cancelled"), "{error}");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        occurrence_times(&observed.lock().unwrap(), marker).len(),
        1,
        "CR executed the echoed command after cancellation"
    );
    submitter.join().unwrap();
    registry::close_card(info.name).expect("close cleanup");
}
