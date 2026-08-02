// Integrationstests for PtyHost — cmd-echo-baserede (spike-mønsteret fra
// ConPTY-spikens harness): sentinel-assertions på ANSI-strippet output.
// Venter ALDRIG på reader-EOF (FUND 11) — exit observeres via try_exit_status.
// Testene kører parallelt; samtidige ConPTY'er er spike-bevist (FUND 9).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use talminal_canvas_lib::pty::{PtyError, PtyHost, PtySpawn};

// ---------- hjælpere ----------

fn cmd_exe() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!(r"{root}\System32\cmd.exe")
}

fn test_cwd() -> PathBuf {
    std::env::temp_dir()
}

fn spawn_collecting(
    command: Vec<String>,
    extra_env: Vec<(String, String)>,
) -> (PtyHost, Arc<Mutex<Vec<u8>>>) {
    // Task 2 (env-politik-split): deny-felterne udfyldes fra CC-profilen —
    // spejler main.rs' spawn-sti, saa testene her ser praecis app'ens env
    // (nettoeffekt identisk med den tidligere hardcodede liste i pty.rs).
    let cc = talminal_canvas_lib::profiles::profile("claude").expect("claude profile");
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buf);
    let host = PtyHost::spawn(
        PtySpawn {
            cwd: test_cwd(),
            command,
            cols: 120,
            rows: 35,
            env_deny_prefixes: cc.env_deny_prefixes.iter().map(|s| s.to_string()).collect(),
            env_deny_exact: cc.env_deny_exact.iter().map(|s| s.to_string()).collect(),
            extra_env,
        },
        move |bytes| sink.lock().unwrap().extend_from_slice(bytes),
    )
    .expect("spawn");
    (host, buf)
}

fn stripped(buf: &Arc<Mutex<Vec<u8>>>) -> String {
    strip_ansi(&buf.lock().unwrap())
}

fn wait_for(buf: &Arc<Mutex<Vec<u8>>>, needle: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if stripped(buf).contains(needle) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_exit(host: &PtyHost, timeout: Duration) -> Option<u32> {
    let start = Instant::now();
    loop {
        if let Some(code) = host.try_exit_status() {
            return Some(code);
        }
        if start.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// ANSI-stripper fra spike-harnessen (kanonisk mønster) — sentinel-søgning
/// må ikke forstyrres af escape-sekvenser i ConPTY-outputtet.
fn strip_ansi(bytes: &[u8]) -> String {
    #[derive(PartialEq)]
    enum St {
        Normal,
        Esc,
        Csi,
        Osc,
        OscEsc,
        Str, // DCS/SOS/PM/APC — til ESC \
        StrEsc,
        EscInter, // ESC ( ) * + — én byte mere
    }
    let mut st = St::Normal;
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        match st {
            St::Normal => {
                if b == 0x1B {
                    st = St::Esc;
                } else if b == b'\r' || b == b'\n' || b == b'\t' || b >= 0x20 {
                    out.push(b);
                }
            }
            St::Esc => {
                st = match b {
                    b'[' => St::Csi,
                    b']' => St::Osc,
                    b'P' | b'X' | b'^' | b'_' => St::Str,
                    b'(' | b')' | b'*' | b'+' => St::EscInter,
                    _ => St::Normal,
                };
            }
            St::Csi => {
                if (0x40..=0x7E).contains(&b) {
                    st = St::Normal;
                }
            }
            St::Osc => {
                if b == 0x07 {
                    st = St::Normal;
                } else if b == 0x1B {
                    st = St::OscEsc;
                }
            }
            St::OscEsc => {
                st = if b == b'\\' { St::Normal } else { St::Osc };
            }
            St::Str => {
                if b == 0x1B {
                    st = St::StrEsc;
                }
            }
            St::StrEsc => {
                st = if b == b'\\' { St::Normal } else { St::Str };
            }
            St::EscInter => st = St::Normal,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------- tests ----------

#[test]
fn spawn_rejects_empty_command() {
    let err = PtyHost::spawn(
        PtySpawn {
            cwd: test_cwd(),
            command: vec![],
            cols: 80,
            rows: 24,
            env_deny_prefixes: vec![],
            env_deny_exact: vec![],
            extra_env: vec![],
        },
        |_: &[u8]| {},
    )
    .err()
    .expect("empty command must fail");
    assert!(matches!(err, PtyError::Spawn(_)));
}

#[test]
fn spawn_echo_sentinel_exit_and_teardown() {
    // DSR-regressionstesten: uden autosvar på ESC[6n hænger selv cmd /c echo
    // totalt (FUND 1, spike run 1: 90 bytes, intet echo, ingen exit).
    let (host, buf) = spawn_collecting(
        vec![cmd_exe(), "/c".into(), "echo CANVAS-PTY-SMOKE-OK".into()],
        vec![],
    );
    assert!(
        wait_for(&buf, "CANVAS-PTY-SMOKE-OK", Duration::from_secs(10)),
        "sentinel not seen; stripped output: {}",
        stripped(&buf)
    );
    assert!(
        wait_exit(&host, Duration::from_secs(10)).is_some(),
        "child exit not observed via try_exit_status"
    );
    assert_eq!(
        host.dsr_replies(),
        1,
        "ConPTY's første ESC[6n skal være besvaret præcis én gang"
    );
    // Fix F1: query-bytes SPLEJSES UD af outputtet — når xterm aldrig ser
    // ESC[6n, kan den hverken auto-svare CPR (falsk auto-pause) eller
    // sende dobbelt-CPR til childen.
    assert!(
        !buf.lock().unwrap().windows(4).any(|w| w == b"\x1b[6n"),
        "the first ESC[6n must be spliced out of on_output (fix F1)"
    );
    host.kill_and_teardown().expect("teardown");
}

#[test]
fn write_reaches_child_and_output_flows_back() {
    let (host, buf) = spawn_collecting(vec![cmd_exe()], vec![]);
    assert!(
        wait_for(&buf, ">", Duration::from_secs(10)),
        "cmd prompt not seen"
    );
    host.write(b"echo PTY-WRITE-ROUNDTRIP-OK\r").expect("write");
    assert!(
        wait_for(&buf, "PTY-WRITE-ROUNDTRIP-OK", Duration::from_secs(10)),
        "roundtrip sentinel not seen; stripped output: {}",
        stripped(&buf)
    );
    host.write(b"exit\r").expect("write exit");
    assert!(wait_exit(&host, Duration::from_secs(10)).is_some());
    host.kill_and_teardown().expect("teardown");
}

#[test]
fn resize_succeeds_and_child_survives() {
    let (host, buf) = spawn_collecting(vec![cmd_exe()], vec![]);
    assert!(
        wait_for(&buf, ">", Duration::from_secs(10)),
        "cmd prompt not seen"
    );
    host.resize(80, 24).expect("resize shrink");
    host.resize(200, 50).expect("resize grow");
    host.write(b"echo AFTER-RESIZE-OK\r").expect("write");
    assert!(
        wait_for(&buf, "AFTER-RESIZE-OK", Duration::from_secs(10)),
        "child dead or unresponsive after resize; stripped output: {}",
        stripped(&buf)
    );
    host.write(b"exit\r").expect("write exit");
    assert!(wait_exit(&host, Duration::from_secs(10)).is_some());
    host.kill_and_teardown().expect("teardown");
}

#[test]
fn kill_and_teardown_terminates_long_running_child() {
    // Kill-vejen (FUND 8/11): TerminateProcess → drop writer → dræn →
    // drop master → join reader. Må aldrig hænge.
    let (host, buf) = spawn_collecting(vec![cmd_exe()], vec![]);
    assert!(
        wait_for(&buf, ">", Duration::from_secs(10)),
        "cmd prompt not seen"
    );
    let t0 = Instant::now();
    host.kill_and_teardown().expect("teardown after kill");
    assert!(
        t0.elapsed() < Duration::from_secs(20),
        "teardown must not hang (took {:?})",
        t0.elapsed()
    );
}

#[test]
fn sequenced_write_watermark_precedes_the_resulting_echo() {
    let cc = talminal_canvas_lib::profiles::profile("claude").expect("claude profile");
    let observed: Arc<Mutex<Vec<(u64, u8)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    let host = PtyHost::spawn_sequenced(
        PtySpawn {
            cwd: test_cwd(),
            command: vec![cmd_exe()],
            cols: 120,
            rows: 35,
            env_deny_prefixes: cc.env_deny_prefixes.iter().map(|s| s.to_string()).collect(),
            env_deny_exact: cc.env_deny_exact.iter().map(|s| s.to_string()).collect(),
            extra_env: vec![],
        },
        move |sequence, bytes| {
            sink.lock()
                .unwrap()
                .extend(bytes.iter().copied().map(|byte| (sequence, byte)));
        },
    )
    .expect("spawn sequenced cmd");

    let prompt_started = Instant::now();
    while !observed
        .lock()
        .unwrap()
        .iter()
        .any(|(_, byte)| *byte == b'>')
    {
        assert!(prompt_started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }

    let marker = b"P5-SEQUENCE-WATERMARK";
    let watermark = host
        .write_if_with_output_watermark(b"echo P5-SEQUENCE-WATERMARK\r", Some)
        .expect("write")
        .expect("admitted write");
    let echo_started = Instant::now();
    loop {
        let entries = observed.lock().unwrap();
        let found = entries.windows(marker.len()).find_map(|window| {
            window
                .iter()
                .map(|(_, byte)| *byte)
                .eq(marker.iter().copied())
                .then_some(window.iter().map(|(sequence, _)| *sequence).max().unwrap())
        });
        drop(entries);
        if let Some(echo_sequence) = found {
            assert!(
                echo_sequence > watermark,
                "echo sequence {echo_sequence} did not follow watermark {watermark}"
            );
            break;
        }
        assert!(echo_started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }

    host.write(b"exit\r").expect("exit write");
    assert!(wait_exit(&host, Duration::from_secs(10)).is_some());
    host.kill_and_teardown().expect("teardown");
}

#[test]
fn teardown_drains_high_output_through_final_sentinel() {
    // Readeren forsinkes bevidst pr. chunk, så child-exit ikke i sig selv
    // betyder, at on_output allerede har set hele ConPTY-halen. Dræningen
    // skal observere hvert bytes_read-hop og bevare den sidste sentinel.
    let cc = talminal_canvas_lib::profiles::profile("claude").expect("claude profile");
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buf);
    let host = PtyHost::spawn(
        PtySpawn {
            cwd: test_cwd(),
            command: vec![
                cmd_exe(),
                "/d".into(),
                "/c".into(),
                "(for /L %i in (1,1,4000) do @echo 0123456789abcdef0123456789abcdef) & echo DRAIN-FINAL-SENTINEL".into(),
            ],
            cols: 120,
            rows: 35,
            env_deny_prefixes: cc.env_deny_prefixes.iter().map(|s| s.to_string()).collect(),
            env_deny_exact: cc.env_deny_exact.iter().map(|s| s.to_string()).collect(),
            extra_env: vec![],
        },
        move |bytes| {
            std::thread::sleep(Duration::from_millis(2));
            sink.lock().unwrap().extend_from_slice(bytes);
        },
    )
    .expect("spawn high-output child");

    // 60 s og ikke 15: deadline'en er et vaern mod at HAENGE, ikke en
    // hastigheds-assertion — testen beviser at draeningen bevarer den sidste
    // sentinel, uanset hvor lang tid barnet bruger. Isoleret koerer den paa
    // 0,69 s, men under fuld parallel suite kan den overskride 15 s (maalt
    // 2026-08-02: faeldede ét ritual, groen 3/3 isoleret i samme minut).
    // Paa en toekernet CI-runner er 15 s en kilde til roed CI uden en fejl,
    // og en gate der er roed uden grund bliver ignoreret.
    assert!(
        wait_exit(&host, Duration::from_secs(60)).is_some(),
        "high-output child did not exit"
    );
    host.kill_and_teardown().expect("drain high-output tail");
    assert!(
        stripped(&buf).contains("DRAIN-FINAL-SENTINEL"),
        "mandatory drain lost the final ConPTY output"
    );
}

#[test]
fn teardown_waits_for_busy_reader_then_drains_buffered_tail() {
    // Regression for the 50 ms drain window: the reader has pulled its first
    // chunk, but its output callback is deliberately blocked for >50 ms while
    // the already-exited child leaves later chunks in ConPTY. Teardown must
    // not call that apparent counter-stability "quiet" or lose the sentinel.
    let cc = talminal_canvas_lib::profiles::profile("claude").expect("claude profile");
    let marker_dir = tempfile::tempdir().expect("temp marker dir");
    let release_path = marker_dir.path().join("release-child.txt");
    let marker_path = marker_dir.path().join("child-finished.txt");
    let powershell =
        PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let command = format!(
        "[Console]::Out.WriteLine('BUSY-DRAIN-FIRST-SENTINEL'); while (-not (Test-Path -LiteralPath '{}')) {{ Start-Sleep -Milliseconds 10 }}; [Console]::Out.WriteLine('BUSY-DRAIN-FINAL-SENTINEL'); Set-Content -LiteralPath '{}' -Value ready",
        release_path.display(),
        marker_path.display()
    );
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buf);
    let block_once = Arc::new(AtomicBool::new(true));
    let block_once_for_reader = Arc::clone(&block_once);
    let release_path_for_reader = release_path.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let host = Arc::new(
        PtyHost::spawn(
            PtySpawn {
                cwd: test_cwd(),
                command: vec![
                    powershell.display().to_string(),
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    command,
                ],
                cols: 120,
                rows: 35,
                env_deny_prefixes: cc.env_deny_prefixes.iter().map(|s| s.to_string()).collect(),
                env_deny_exact: cc.env_deny_exact.iter().map(|s| s.to_string()).collect(),
                extra_env: vec![],
            },
            move |bytes| {
                let should_block = {
                    let mut output = sink.lock().unwrap();
                    output.extend_from_slice(bytes);
                    block_once_for_reader.load(Ordering::SeqCst)
                        && String::from_utf8_lossy(&output).contains("BUSY-DRAIN-FIRST-SENTINEL")
                };
                if should_block && block_once_for_reader.swap(false, Ordering::SeqCst) {
                    std::fs::write(&release_path_for_reader, b"go")
                        .expect("release child to write buffered tail");
                    entered_tx.send(()).expect("announce blocked callback");
                    release_rx
                        .recv_timeout(Duration::from_secs(15))
                        .expect("release blocked callback");
                }
            },
        )
        .expect("spawn buffered-tail child"),
    );

    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("reader callback was not entered");
    let marker_deadline = Instant::now() + Duration::from_secs(5);
    while !marker_path.is_file() && Instant::now() < marker_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !marker_path.is_file() {
        let _ = release_tx.send(());
        let _ = host.kill_and_teardown();
        panic!("child did not write the post-tail marker while callback was blocked");
    }
    if wait_exit(&host, Duration::from_secs(5)).is_none() {
        let _ = release_tx.send(());
        let _ = host.kill_and_teardown();
        panic!("child did not exit after writing its buffered tail");
    }

    let teardown_host = Arc::clone(&host);
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        done_tx
            .send(teardown_host.kill_and_teardown())
            .expect("report teardown result");
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        done_rx.try_recv().is_err(),
        "teardown treated an active output callback as 50 ms quiescence"
    );
    release_tx.send(()).expect("release reader callback");
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("teardown did not finish after callback release")
        .expect("teardown after busy reader");
    assert!(
        stripped(&buf).contains("BUSY-DRAIN-FINAL-SENTINEL"),
        "mandatory drain lost output buffered behind a busy callback"
    );
}

#[test]
fn extra_env_is_set_and_inherited_claude_vars_are_scrubbed() {
    let (host, buf) = spawn_collecting(
        vec![cmd_exe(), "/c".into(), "set".into()],
        vec![("TALMINAL_SESSION_ID".into(), "card-a".into())],
    );
    assert!(
        wait_for(&buf, "TALMINAL_SESSION_ID=card-a", Duration::from_secs(10)),
        "extra_env not visible in child env; stripped output: {}",
        stripped(&buf)
    );
    assert!(wait_exit(&host, Duration::from_secs(10)).is_some());
    // giv reader-tråden et øjeblik til at dræne resten før fraværs-asserts
    std::thread::sleep(Duration::from_millis(300));
    let text = stripped(&buf);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("CLAUDE")) {
        assert!(
            !text.contains(&format!("\n{k}=")),
            "inherited {k} must be scrubbed from child env"
        );
    }
    host.kill_and_teardown().expect("teardown");
}

// ---------- Task 1: frossen IPC-stub-kontrakt (default-state, supervision OFF) ----------
//
// Stub-semantikken (bindende for alle senere tasks, plan Task 1):
//   - gaten er ALTID pass-through: alle epoch/source-vaerdier accepteres,
//     bytes skrives altid (aldrig "stale_epoch")
//   - kort-state: owner=="persona", epoch==0
//   - ingen pause-/resume-signalfiler skrives nogensinde
// Beslutningen bor i lib-cratet (control-facaden), saa testene naar den her —
// main.rs-wrapperne er tynde og kalder praecis disse funktioner.

#[cfg(not(feature = "supervision"))]
mod supervision_stub_contract {
    use super::*;
    use talminal_canvas_lib::control::{self, EpochGate, WriteOutcome};

    #[test]
    fn stale_epoch_999_write_still_reaches_child() {
        // (a) given et spawnet kort, when write_pty-beslutningen tages med
        // vilkaarlig stale epoch=999, then skrives bytes — aldrig "stale_epoch".
        // Spejler main.rs' write_pty: beslutning via facaden, derefter host.write.
        let (host, buf) = spawn_collecting(vec![cmd_exe()], vec![]);
        assert!(
            wait_for(&buf, ">", Duration::from_secs(10)),
            "cmd prompt not seen"
        );
        let gate = EpochGate::new();
        match control::gate_pty_write(&gate, "persona", 999) {
            WriteOutcome::Write => {}
            WriteOutcome::RejectStale => panic!("stub gate must never return stale_epoch"),
            WriteOutcome::WriteAfterPause { .. } => panic!("stub gate must never pause"),
        }
        host.write(b"echo STUB-GATE-PASSTHROUGH-OK\r")
            .expect("write");
        assert!(
            wait_for(&buf, "STUB-GATE-PASSTHROUGH-OK", Duration::from_secs(10)),
            "bytes must reach the child after a stale-epoch decision; output: {}",
            stripped(&buf)
        );
        host.write(b"exit\r").expect("write exit");
        assert!(wait_exit(&host, Duration::from_secs(10)).is_some());
        host.kill_and_teardown().expect("teardown");
    }

    #[test]
    fn card_state_is_persona_epoch_zero() {
        // (b) get_card_state's stub-felter: owner "persona", epoch 0 — ogsaa
        // EFTER menneske-input (der i supervision-state ville bumpe epoch).
        let gate = EpochGate::new();
        let _ = control::gate_pty_write(&gate, "human", 3);
        assert_eq!(control::card_owner(&gate), "persona");
        assert_eq!(control::card_epoch(&gate), 0);
    }

    #[test]
    fn human_write_path_creates_no_signal_files() {
        // (c) I supervision-state skriver den foerste menneske-tast en
        // pause-signalfil i <home>\signals\. Default-state: fil-vejen findes
        // slet ikke — signals-dirren maa aldrig opstaa.
        let home = std::env::temp_dir().join(format!("talminal-stub-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("TALMINAL_HOME", &home);
        let gate = EpochGate::new();
        for source in ["human", "persona", "terminal", "wat"] {
            match control::gate_pty_write(&gate, source, 7) {
                WriteOutcome::Write => {}
                _ => panic!("default-state gate must pass through for source={source}"),
            }
        }
        assert_eq!(
            control::card_epoch(&gate),
            0,
            "human input must not bump the epoch in default-state"
        );
        assert!(
            !home.join("signals").exists(),
            "no signal dir/file may be created in default-state"
        );
        std::env::remove_var("TALMINAL_HOME");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn resume_control_is_noop_ok() {
        // Frossen kontrakt: resume_card_control er en no-op der returnerer
        // Ok(()) — intet bump, ingen signalfil, intet event.
        let gate = EpochGate::new();
        assert_eq!(control::resume_control(&gate), Ok(()));
        assert_eq!(control::card_epoch(&gate), 0);
    }
}

// Supervision-state: facaden skal delegere UAENDRET til den rigtige gate —
// de dybe semantik-tests bor i epoch::tests/signals::tests (uaendrede).
#[cfg(feature = "supervision")]
mod supervision_facade {
    use talminal_canvas_lib::control::{self, WriteOutcome};
    use talminal_canvas_lib::epoch::EpochGate;

    #[test]
    fn facade_pauses_on_first_human_key_and_rejects_stale_persona() {
        let dir =
            std::env::temp_dir().join(format!("talminal-facade-signals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let gate = EpochGate::new();
        // Foerste menneske-tast: auto-pause → WriteAfterPause + pause-signalfil.
        match control::gate_pty_write(&gate, &dir, "card-x", "human", 0) {
            WriteOutcome::WriteAfterPause { new_epoch } => assert_eq!(new_epoch, 1),
            _ => panic!("first human key must pause-then-pass"),
        }
        assert!(
            dir.join("card-x.pause.json").exists(),
            "pause signal file must be written"
        );
        // In-flight persona-write med gammel epoch afvises mekanisk (regel 4).
        assert!(matches!(
            control::gate_pty_write(&gate, &dir, "card-x", "persona", 0),
            WriteOutcome::RejectStale
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn credential_env_denylist_is_scrubbed() {
    // Fix F5 (spec §3 'minimalt miljø' — v0 = deny-liste): testen sætter SELV
    // credentials i parent-env FØR spawn, så asserten ALTID er skarp — modsat
    // CLAUDE*-fraværs-asserten ovenfor, der er vakuøs uden for en CC-session.
    // Task 2 (env-politik-split): deny-listerne er ikke længere hardcodet i
    // pty.rs — spawn_collecting sender CC-PROFILENS deny-lister (prefix +
    // exact) som PtySpawn-felter, præcis som main.rs' spawn-sti.
    std::env::set_var("OPENAI_API_KEY", "sk-test-must-never-reach-child");
    std::env::set_var("GITHUB_TOKEN", "ghp-test-must-never-reach-child");
    let (host, buf) = spawn_collecting(
        vec![cmd_exe(), "/c".into(), "set".into()],
        vec![("TALMINAL_SESSION_ID".into(), "card-env".into())],
    );
    assert!(
        wait_for(
            &buf,
            "TALMINAL_SESSION_ID=card-env",
            Duration::from_secs(10)
        ),
        "child env dump not seen; stripped output: {}",
        stripped(&buf)
    );
    assert!(wait_exit(&host, Duration::from_secs(10)).is_some());
    std::thread::sleep(Duration::from_millis(300));
    let text = stripped(&buf);
    for k in ["OPENAI_API_KEY", "GITHUB_TOKEN"] {
        assert!(
            !text.contains(&format!("\n{k}=")),
            "denylisted {k} must be scrubbed from child env"
        );
    }
    host.kill_and_teardown().expect("teardown");
}
