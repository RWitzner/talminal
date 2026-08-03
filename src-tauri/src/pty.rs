// PtyHost — eneste pty-ejer i Talminal (spec v0.6 §5).
// Kanonisk mønster fra ConPTY-spikens harness; dens fund er gengivet her:
//
//   FUND 1  — portable-pty 0.9.0 sætter PSEUDOCONSOLE_INHERIT_CURSOR, så ConPTY
//             åbner med ESC[6n (cursor-position-query) og blokerer ALT indtil
//             værten svarer. Reader-tråden besvarer selv den FØRSTE query med
//             ESC[1;1R — deterministisk, uafhængigt af webview-timing — og
//             SPLEJSER query-bytes UD af outputtet (fix F1): ser xterm.js aldrig
//             query'en, kan den hverken auto-svare CPR via onData (falsk
//             auto-pause) eller sende et ekstra CPR til childen.
//   FUND 4  — flow control er blocking backpressure: reader-tråden dræner ALTID;
//             en pauset reader blokerer workeren.
//   FUND 11 — ingen reader-EOF ved child-exit; EOF kommer først efter master-drop.
//             Teardown-rækkefølge NORMATIV:
//             kill child → drop writer → dræn → drop master → join reader.
//             Vent ALDRIG på reader-EOF som exit-signal.
//
// Fix F2: kill_and_teardown tager &self (interne Option-takes, idempotent);
// kill går via child-Mutex'en — ADSKILT fra writer-Mutex'en, så en hængende
// write aldrig spærrer kill. Fix F4: defensiv Drop som sidste værn ved app-exit.
// Fix F19: reader/writer tages fra master FØR spawn_command — ingen efterladt
// child ved setup-fejl. Fix F5 (Task 2-split): env-skrub = ubetinget
// nested-værn (profiles::NESTED_SCRUB) + per-profil credential-deny-lister
// (PtySpawn.env_deny_prefixes/env_deny_exact).

use std::borrow::Cow;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

const DRAIN_QUIET_WINDOW: Duration = Duration::from_millis(50);
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(5);
const DRAIN_MAX_DURATION: Duration = Duration::from_secs(5);

/// Ren tidslinje for den obligatoriske post-kill-dræning. Nye bytes flytter
/// altid stilhedsdeadline; kalderen venter aldrig på reader-EOF.
struct DrainProgress {
    last_len: usize,
    last_change: Duration,
}

impl DrainProgress {
    fn new(initial_len: usize) -> Self {
        Self {
            last_len: initial_len,
            last_change: Duration::ZERO,
        }
    }

    fn quiet_at(&mut self, len: usize, reader_busy: bool, elapsed: Duration) -> bool {
        // `bytes_read` tælles før output-callbacken. En stabil counter er
        // derfor ikke alene stilhed: callbacken kan stadig holde readeren fra
        // at hente allerede bufferet ConPTY-output. Busy nulstiller vinduet,
        // så masteren aldrig droppes midt i en aktiv output-levering.
        if len != self.last_len || reader_busy {
            self.last_len = len;
            self.last_change = elapsed;
        }
        elapsed.saturating_sub(self.last_change) >= DRAIN_QUIET_WINDOW
    }
}

/// Fejltype for pty-operationer. Tauri-kommandoer (Task 8) mapper med `.to_string()`.
#[derive(Debug)]
pub enum PtyError {
    /// Spawn-fasen fejlede (tom kommando, openpty, spawn_command, reader/writer-opsætning).
    Spawn(String),
    /// I/O-fejl på en levende pty (write/flush/resize) eller reader-panic ved join.
    Io(String),
    /// Pty'en er lukket (writer droppet under teardown).
    Closed,
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PtyError::Spawn(e) => write!(f, "pty spawn failed: {e}"),
            PtyError::Io(e) => write!(f, "pty io error: {e}"),
            PtyError::Closed => write!(f, "pty is closed"),
        }
    }
}

impl std::error::Error for PtyError {}

/// Spawn-spec (skelettets kanoniske form). `extra_env` sættes EFTER
/// env-skrubben (nested-værn + deny-listerne, fix F5) — Task 8 sætter fx
/// ("TALMINAL_SESSION_ID", kortnavn).
///
/// Env-politik-split (Task 2): credential-deny-listen er en PARAMETER
/// (udfyldes fra agent-profilen, profiles.rs) i to lag — prefix + exact.
/// To-lags-semantikken SKAL bevares: flad exact-match ville tavst lække
/// OPENAI_*/AWS_*/VERCEL_*. Nested-værnet (profiles::NESTED_SCRUB) anvendes
/// UBETINGET i spawn — uafhængigt af felterne.
pub struct PtySpawn {
    pub cwd: PathBuf,
    pub command: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    /// Prefix-laget af credential-deny-listen (prefix-match!).
    pub env_deny_prefixes: Vec<String>,
    /// Exact-laget af credential-deny-listen.
    pub env_deny_exact: Vec<String>,
    pub extra_env: Vec<(String, String)>,
}

/// Første-DSR-splejsning (fix F1) som REN, unit-testet tilstandsmaskine:
/// scanner rullende hale + ny chunk for det FØRSTE ESC[6n, melder om der skal
/// svares, og returnerer de bytes der må emittes — de 4 query-bytes splejses
/// UD, og op til 3 trailing bytes tilbageholdes (kan være starten på en split
/// query hen over en chunk-grænse), indtil query'en er fundet. Efter svaret er
/// filtret transparent pass-through.
struct DsrFilter {
    tail: Vec<u8>,
    answered: bool,
}

impl DsrFilter {
    fn new() -> Self {
        Self {
            tail: Vec::new(),
            answered: false,
        }
    }

    /// Returnerer (bytes til on_output, skal-der-svares-nu).
    ///
    /// `Cow` og ikke `Vec`: efter svaret ER filtret transparent pass-through,
    /// men en `to_vec()` her kostede alligevel en allokering + memcpy af HVER
    /// chunk i hele PTY'ens levetid — paa den reader-traad der ifoelge modulets
    /// FUND 4 altid skal draene. Laanet er gratis; kun de to splejse-veje,
    /// som kun loeber indtil den foerste DSR er besvaret, allokerer.
    fn push<'a>(&mut self, input: &'a [u8]) -> (Cow<'a, [u8]>, bool) {
        if self.answered {
            return (Cow::Borrowed(input), false);
        }
        let mut scan = std::mem::take(&mut self.tail);
        scan.extend_from_slice(input);
        if let Some(pos) = scan.windows(4).position(|w| w == b"\x1b[6n") {
            self.answered = true;
            scan.drain(pos..pos + 4); // splejs query-bytes UD af outputtet
            (Cow::Owned(scan), true)
        } else {
            let keep = scan.len().saturating_sub(3);
            self.tail = scan.split_off(keep);
            (Cow::Owned(scan), false)
        }
    }

    /// EOF/fejl: flush en evt. tilbageholdt hale, så ingen bytes tabes.
    fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.tail)
    }
}

/// Én ConPTY pr. kort. `Send + Sync` (alle mutable felter bag egne Mutex'er) —
/// bor i AppState bag `Arc` (Task 8, fix F2), så write/kill kan ske uden at
/// holde kort-låsen.
pub struct PtyHost {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    /// Childens OS-PID, fanget ved spawn (fix F17) — Task 8 logger den, så
    /// run-bookens kill-instruktioner rammer DOKUMENTEREDE worker-PID'er.
    child_pid: Option<u32>,
    reader_thread: Mutex<Option<thread::JoinHandle<()>>>,
    bytes_read: Arc<AtomicUsize>,
    /// Monotonic sequence assigned immediately after the OS read returns and
    /// before filtering, DSR handling, or callback dispatch.
    output_sequence: Arc<AtomicU64>,
    /// Sand mens reader-tråden leverer en allerede læst chunk til
    /// `on_output`. Drain må ikke kalde dette quiescens: der kan ligge flere
    /// bytes i ConPTY-bufferen, som først læses når callbacken returnerer.
    reader_busy: Arc<AtomicBool>,
    reader_eof: Arc<AtomicBool>,
    dsr_replies: Arc<AtomicUsize>,
}

impl PtyHost {
    /// Spawner `spec.command` i en ny ConPTY. `on_output` kaldes fra
    /// reader-tråden med hver rå byte-chunk (Task 8 base64'er til "pty-output").
    pub fn spawn(
        spec: PtySpawn,
        on_output: impl Fn(&[u8]) + Send + 'static,
    ) -> Result<PtyHost, PtyError> {
        Self::spawn_sequenced(spec, move |_sequence, bytes| on_output(bytes))
    }

    /// Variant used by readiness-gated submit. The sequence is assigned in
    /// the reader before the callback, so delayed pre-write callbacks retain
    /// their pre-write identity.
    pub fn spawn_sequenced(
        spec: PtySpawn,
        on_output: impl Fn(u64, &[u8]) + Send + 'static,
    ) -> Result<PtyHost, PtyError> {
        let program = spec
            .command
            .first()
            .ok_or_else(|| PtyError::Spawn("empty command".into()))?;
        let mut cmd = CommandBuilder::new(program);
        for arg in &spec.command[1..] {
            cmd.arg(arg);
        }
        cmd.cwd(&spec.cwd);
        // Env-skrub (fix F5, spec §3 "minimalt miljø" — v0 = deny-liste),
        // Task 2-split i to uafhængige værn:
        //   1) Nested-værn (profiles::NESTED_SCRUB): UBETINGET — anvendes
        //      også med tomme deny-felter (spike-mønster: kort-CC må ikke
        //      se sig som nested child-session).
        //   2) Credential-deny per agent-profil: prefix- + exact-laget fra
        //      spec.env_deny_* (CC-profilen = pty.rs' oprindelige lister
        //      verbatim). To-lags-semantikken SKAL bevares.
        // Bevidst IKKE env_clear()+allowlist (låst afgørelse: OAuth-arven
        // §20.3 er ejer-beslutning — credentials bor i brugerens
        // config-filer; CC kræver mange Windows-vars). Fuldt minimalt env =
        // M2.5; residualet (øvrige ambiente vars) er journalført.
        for (k, _) in std::env::vars() {
            if crate::profiles::nested_scrub_matches(&k)
                || spec
                    .env_deny_prefixes
                    .iter()
                    .any(|p| k.starts_with(p.as_str()))
                || spec.env_deny_exact.iter().any(|e| k == e.as_str())
            {
                cmd.env_remove(&k);
            }
        }
        // Eksplicitte envs sættes EFTER skrubben.
        for (k, v) in &spec.extra_env {
            cmd.env(k, v);
        }

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Spawn(format!("openpty: {e}")))?;
        // Fix F19: reader/writer tages fra pair.master FØR spawn_command —
        // begge kald er master-lokale og uafhængige af spawnen. Fejlede de
        // EFTER spawnen, ville ?-early-return efterlade en kørende child
        // uden teardown (portable-pty's Child-drop dræber ikke processen).
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Spawn(format!("clone reader: {e}")))?;
        let writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> = Arc::new(Mutex::new(Some(
            pair.master
                .take_writer()
                .map_err(|e| PtyError::Spawn(format!("take writer: {e}")))?,
        )));
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Spawn(format!("spawn_command: {e}")))?;
        let child_pid = child.process_id(); // fix F17
        drop(pair.slave);

        let bytes_read = Arc::new(AtomicUsize::new(0));
        let output_sequence = Arc::new(AtomicU64::new(0));
        let reader_busy = Arc::new(AtomicBool::new(false));
        let reader_eof = Arc::new(AtomicBool::new(false));
        let dsr_replies = Arc::new(AtomicUsize::new(0));

        let writer2 = Arc::clone(&writer);
        let bytes2 = Arc::clone(&bytes_read);
        let sequence2 = Arc::clone(&output_sequence);
        let busy2 = Arc::clone(&reader_busy);
        let eof2 = Arc::clone(&reader_eof);
        let dsr2 = Arc::clone(&dsr_replies);
        let reader_thread = thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            // Fix F1: DsrFilter konsumerer den FØRSTE ESC[6n chunk-sikkert —
            // query-bytes splejses UD og når ALDRIG on_output/xterm; op til
            // 3 trailing bytes tilbageholdes indtil query'en er fundet
            // (split-over-chunk-grænse-tilfældet), og flushes ved EOF/fejl.
            let mut dsr = DsrFilter::new();
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => {
                        let rest = dsr.flush();
                        if !rest.is_empty() {
                            bytes2.fetch_add(rest.len(), Ordering::SeqCst);
                            let sequence = sequence2.fetch_add(1, Ordering::SeqCst) + 1;
                            busy2.store(true, Ordering::SeqCst);
                            on_output(sequence, &rest);
                            busy2.store(false, Ordering::SeqCst);
                        }
                        eof2.store(true, Ordering::SeqCst);
                        break;
                    }
                    Ok(n) => {
                        // Assign the read identity immediately after the OS
                        // read returns, before filtering or a DSR writer
                        // roundtrip can yield to a concurrent submit.
                        let sequence = sequence2.fetch_add(1, Ordering::SeqCst) + 1;
                        let (out, reply_now) = dsr.push(&chunk[..n]);
                        if reply_now {
                            if let Ok(mut guard) = writer2.lock() {
                                if let Some(w) = guard.as_mut() {
                                    let _ = w.write_all(b"\x1b[1;1R");
                                    let _ = w.flush();
                                    dsr2.fetch_add(1, Ordering::SeqCst);
                                }
                            }
                        }
                        if !out.is_empty() {
                            // Tæl straks efter read/filter, FØR callbacken.
                            // Teardown kan dermed skelne en aktiv callback fra
                            // manglende reader-fremdrift via `reader_busy`.
                            bytes2.fetch_add(out.len(), Ordering::SeqCst);
                            busy2.store(true, Ordering::SeqCst);
                            on_output(sequence, &out);
                            busy2.store(false, Ordering::SeqCst);
                        }
                    }
                    Err(_) => {
                        let rest = dsr.flush();
                        if !rest.is_empty() {
                            bytes2.fetch_add(rest.len(), Ordering::SeqCst);
                            let sequence = sequence2.fetch_add(1, Ordering::SeqCst) + 1;
                            busy2.store(true, Ordering::SeqCst);
                            on_output(sequence, &rest);
                            busy2.store(false, Ordering::SeqCst);
                        }
                        eof2.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });

        Ok(PtyHost {
            master: Mutex::new(Some(pair.master)),
            writer,
            child: Mutex::new(child),
            child_pid,
            reader_thread: Mutex::new(Some(reader_thread)),
            bytes_read,
            output_sequence,
            reader_busy,
            reader_eof,
            dsr_replies,
        })
    }

    /// Skriver bytes til pty'ens input (menneske-tast eller persona-dispatch).
    pub fn write(&self, bytes: &[u8]) -> Result<(), PtyError> {
        let mut guard = self.writer.lock().map_err(|_| PtyError::Closed)?;
        let w = guard.as_mut().ok_or(PtyError::Closed)?;
        w.write_all(bytes)
            .map_err(|e| PtyError::Io(e.to_string()))?;
        w.flush().map_err(|e| PtyError::Io(e.to_string()))
    }

    /// Snapshots the reader sequence and arms caller state under the writer
    /// mutex, immediately before the bytes are written. `None` rejects the
    /// write without touching ConPTY.
    pub fn write_if_with_output_watermark<T>(
        &self,
        bytes: &[u8],
        arm: impl FnOnce(u64) -> Option<T>,
    ) -> Result<Option<T>, PtyError> {
        let mut guard = self.writer.lock().map_err(|_| PtyError::Closed)?;
        let w = guard.as_mut().ok_or(PtyError::Closed)?;
        let Some(token) = arm(self.output_sequence.load(Ordering::SeqCst)) else {
            return Ok(None);
        };
        w.write_all(bytes)
            .map_err(|e| PtyError::Io(e.to_string()))?;
        w.flush().map_err(|e| PtyError::Io(e.to_string()))?;
        Ok(Some(token))
    }

    /// Resizer ConPTY'en (xterm.js-fit → resize_pty, Task 8/9). Robust under
    /// serier af shrink/grow (spike FUND 5).
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        let guard = self.master.lock().map_err(|_| PtyError::Closed)?;
        let master = guard.as_ref().ok_or(PtyError::Closed)?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Io(e.to_string()))
    }

    /// Childens OS-PID fanget ved spawn (fix F17) — None hvis portable-pty
    /// ikke leverede den. Task 8 logger den ved spawn til run-bookens
    /// kill-verifikation.
    pub fn process_id(&self) -> Option<u32> {
        self.child_pid
    }

    /// Samlet antal bytes leveret til `on_output` af reader-tråden (tælles
    /// UANSET hvad closuren gør med dem). Task 8's gating-test beviser med
    /// denne at læsningen FORTSÆTTER mens et kort er skjult — output-gating
    /// sidder i emit-closuren (registry::gated_emit), aldrig i read-løkken,
    /// så backpressure ikke kan ramme child'en.
    pub fn bytes_read(&self) -> usize {
        self.bytes_read.load(Ordering::SeqCst)
    }

    pub fn output_sequence(&self) -> u64 {
        self.output_sequence.load(Ordering::SeqCst)
    }

    /// Ikke-blokerende exit-check: `Some(exit_code)` når child er exitet.
    /// Task 8 poller denne til "card-exit"-eventet. Reader-EOF må ALDRIG
    /// bruges som exit-signal (FUND 11: EOF kommer først efter master-drop).
    pub fn try_exit_status(&self) -> Option<u32> {
        let mut child = self.child.lock().ok()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code()),
            _ => None,
        }
    }

    /// Antal DSR-autosvar sendt (0 eller 1 — kun FØRSTE query besvares i
    /// Rust, og dens bytes SPLEJSES UD af outputtet (fix F1), så xterm.js
    /// aldrig ser den. Eventuelle SENERE queries flyder uændret til xterm;
    /// dens auto-CPR-svar returneres via onData→write_pty med
    /// source:"terminal" — aldrig som menneske-input, aldrig som pause).
    pub fn dsr_replies(&self) -> usize {
        self.dsr_replies.load(Ordering::SeqCst)
    }

    /// Nedlukning i NORMATIV rækkefølge (FUND 11):
    /// 1) kill child (hvis stadig levende) og afvent exit,
    /// 2) drop writer, 3) dræn til quiescens (ALDRIG EOF-vent),
    /// 4) drop master (lukker ConPTY → reader får EOF), 5) join reader.
    ///
    /// Fix F2: tager &self med interne Option-takes — IDEMPOTENT (andet kald
    /// er no-op), og kill går via child-Mutex'en, som er ADSKILT fra
    /// writer-Mutex'en: en hængende write kan aldrig spærre kill/teardown.
    pub fn kill_and_teardown(&self) -> Result<(), PtyError> {
        #[cfg(feature = "perf-trace")]
        let total_started = Instant::now();
        // 1) kill child + afvent exit (max 10 s; teardown fortsætter uanset)
        #[cfg(feature = "perf-trace")]
        let child_lock_started = Instant::now();
        if let Ok(mut child) = self.child.lock() {
            #[cfg(feature = "perf-trace")]
            let child_lock_wait_ms = child_lock_started.elapsed().as_secs_f64() * 1_000.0;
            let already_exited = !matches!(child.try_wait(), Ok(None));
            #[cfg(feature = "perf-trace")]
            let mut kill_requested = false;
            if !already_exited {
                let _ = child.kill();
                crate::perf_only!({
                    kill_requested = true;
                });
            }
            #[cfg(feature = "perf-trace")]
            let exit_wait_started = Instant::now();
            let deadline = Instant::now() + Duration::from_secs(10);
            #[cfg(feature = "perf-trace")]
            let mut timed_out = false;
            while !matches!(child.try_wait(), Ok(Some(_))) {
                if Instant::now() >= deadline {
                    crate::perf_only!({
                        timed_out = true;
                    });
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            crate::perf_mark!(
                "close.pty.child_exit.end",
                serde_json::json!({
                    "child_lock_wait_ms": child_lock_wait_ms,
                    "already_exited": already_exited,
                    "kill_requested": kill_requested,
                    "exit_wait_ms": exit_wait_started.elapsed().as_secs_f64() * 1_000.0,
                    "timed_out": timed_out,
                }),
            );
        }
        // 2) drop writer (lukker input-siden — Option gør droppet reelt)
        #[cfg(feature = "perf-trace")]
        let writer_started = Instant::now();
        if let Ok(mut guard) = self.writer.lock() {
            *guard = None;
        }
        crate::perf_mark!(
            "close.pty.writer_drop.end",
            serde_json::json!({
                "duration_ms": writer_started.elapsed().as_secs_f64() * 1_000.0,
            }),
        );
        // 3) dræn: vent på 50 ms stilhed (max 5 s) — IKKE på EOF
        // (FUND 11). Del A viste 303,9–304,6 ms uden et eneste reset; den
        // gamle 300 ms-konstant var derfor hele normal-pathens ventetid.
        let drain_start = Instant::now();
        let bytes_at_start = self.bytes_read.load(Ordering::SeqCst);
        let mut progress = DrainProgress::new(bytes_at_start);
        #[cfg(feature = "perf-trace")]
        let mut drain_reason = "eof";
        while !self.reader_eof.load(Ordering::SeqCst) {
            let len = self.bytes_read.load(Ordering::SeqCst);
            let reader_busy = self.reader_busy.load(Ordering::SeqCst);
            let elapsed = drain_start.elapsed();
            if progress.quiet_at(len, reader_busy, elapsed) || elapsed >= DRAIN_MAX_DURATION {
                crate::perf_only!({
                    drain_reason = if elapsed >= DRAIN_MAX_DURATION {
                        "timeout"
                    } else {
                        "quiet"
                    };
                });
                break;
            }
            thread::sleep(DRAIN_POLL_INTERVAL);
        }
        #[cfg(feature = "perf-trace")]
        let bytes_at_end = self.bytes_read.load(Ordering::SeqCst);
        crate::perf_mark!(
            "close.pty.drain.end",
            serde_json::json!({
                "duration_ms": drain_start.elapsed().as_secs_f64() * 1_000.0,
                "reason": drain_reason,
                "bytes_read": bytes_at_end,
                "bytes_at_start": bytes_at_start,
                "bytes_drained": bytes_at_end.saturating_sub(bytes_at_start),
            }),
        );
        // 4) drop master — lukker ConPTY (24H2: ClosePseudoConsole returnerer
        //    øjeblikkeligt, FUND 8) og giver reader-tråden EOF
        #[cfg(feature = "perf-trace")]
        let master_started = Instant::now();
        if let Ok(mut master) = self.master.lock() {
            drop(master.take());
        }
        crate::perf_mark!(
            "close.pty.master_drop.end",
            serde_json::json!({
                "duration_ms": master_started.elapsed().as_secs_f64() * 1_000.0,
            }),
        );
        // 5) join reader-tråden
        let handle = match self.reader_thread.lock() {
            Ok(mut rt) => rt.take(),
            Err(_) => None,
        };
        #[cfg(feature = "perf-trace")]
        let join_started = Instant::now();
        if let Some(handle) = handle {
            handle
                .join()
                .map_err(|_| PtyError::Io("reader thread panicked".into()))?;
        }
        crate::perf_mark!(
            "close.pty.reader_join.end",
            serde_json::json!({
                "duration_ms": join_started.elapsed().as_secs_f64() * 1_000.0,
                "total_ms": total_started.elapsed().as_secs_f64() * 1_000.0,
            }),
        );
        Ok(())
    }
}

impl Drop for PtyHost {
    /// Defensivt sidste værn (fix F4): best-effort teardown i korrekt
    /// felt-rækkefølge, hvis kill_and_teardown ikke allerede er kørt —
    /// idempotent via de samme Option-felter (no-op efter gennemført
    /// teardown). Default-drop ville ellers droppe master FØRST (mod den
    /// normative rækkefølge) og droppe JoinHandle uden join. JoinHandle har
    /// ingen timed join: der polles is_finished() mod en deadline, ellers
    /// detaches (ufarligt efter master-drop — reader får EOF).
    fn drop(&mut self) {
        // 1) kill child hvis stadig levende (bounded wait)
        if let Ok(mut child) = self.child.lock() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
                let deadline = Instant::now() + Duration::from_secs(2);
                while !matches!(child.try_wait(), Ok(Some(_))) {
                    if Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
        // 2) drop writer
        if let Ok(mut guard) = self.writer.lock() {
            *guard = None;
        }
        // 3) drop master (giver reader-tråden EOF)
        if let Ok(mut master) = self.master.lock() {
            drop(master.take());
        }
        // 4) join reader med deadline — ellers detach
        if let Ok(mut rt) = self.reader_thread.lock() {
            if let Some(handle) = rt.take() {
                let deadline = Instant::now() + Duration::from_secs(2);
                while !handle.is_finished() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(25));
                }
                if handle.is_finished() {
                    let _ = handle.join();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{DrainProgress, DsrFilter};

    #[test]
    fn dsr_split_over_chunk_boundary_is_spliced_out_and_answered_once() {
        // Fix F1-testkrav (i): split-DSR over chunk-grænse ⇒ præcis ét svar,
        // query-bytes fraværende i output.
        let mut f = DsrFilter::new();
        let (out1, reply1) = f.push(b"AB\x1b[");
        assert_eq!(&*out1, b"A"); // 3-byte-halen tilbageholdes
        assert!(!reply1);
        let (out2, reply2) = f.push(b"6nCD");
        assert_eq!(&*out2, b"BCD"); // query-bytes splejset UD
        assert!(reply2); // præcis ét svar
        let (out3, reply3) = f.push(b"\x1b[6n");
        assert_eq!(&*out3, b"\x1b[6n"); // senere queries røres IKKE (xterm/CC-domæne)
        assert!(!reply3);
    }

    #[test]
    fn dsr_filter_flush_emits_withheld_tail() {
        let mut f = DsrFilter::new();
        let (out, replied) = f.push(b"XYZ");
        assert_eq!(&*out, b"");
        assert!(!replied);
        assert_eq!(f.flush(), b"XYZ");
        assert_eq!(f.flush(), b""); // idempotent
    }

    #[test]
    fn drain_quiet_deadline_resets_on_new_bytes() {
        let mut progress = DrainProgress::new(100);
        assert!(!progress.quiet_at(100, false, Duration::from_millis(49)));
        assert!(!progress.quiet_at(120, false, Duration::from_millis(50)));
        assert!(!progress.quiet_at(120, false, Duration::from_millis(99)));
        assert!(!progress.quiet_at(120, true, Duration::from_millis(100)));
        assert!(!progress.quiet_at(120, false, Duration::from_millis(149)));
        assert!(progress.quiet_at(120, false, Duration::from_millis(150)));
    }
}
