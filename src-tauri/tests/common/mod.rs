//! Delt testhjaelp for alle threads_*-targets. Filen er ikke selv et
//! test-target (kun toplevel-.rs i tests/ er), saa hver testbinary faar sin
//! EGEN kopi af statics herinde — praecis det scope serialiseringen skal have.
//!
//! DATAMAPPEN ER PROCES-GLOBAL TILSTAND, praecis som registryet er det.
//! `talminal_base()` opløser sin sti paa SKRIVETIDSPUNKTET fra env, og
//! default'en er ejerens levende `%LOCALAPPDATA%\Talminal`. En test der
//! glemte at pege den et andet sted skrev derfor i den rigtige datamappe —
//! tavst, uden at fejle, saa intet fortalte det. Maalt paa den fulde suite
//! (begge feature-states) var skaden `threads\t1.jsonl` +32,8 KB og
//! `t2.jsonl` +1,6 KB pr. koersel.
//!
//! Appen selv laeser IKKE det affald: `main.rs` peger `TALMINAL_HOME` paa
//! `projects\<slug>` foer `terminalize_awaiting_on_startup()` kaldes, saa
//! appens arkiv bor et andet sted. Men roden holder LEVENDE global tilstand —
//! `settings.json` opløses af `project::global_base()` og bor praecis her — saa
//! at det hidtil kun var `threads\` der blev ramt er et tilfaelde, ikke en
//! beskyttelse.
//!
//! Derfor sandkasses datamappen HER frem for i hver enkelt test: `serial()`
//! er allerede kontrakten "jeg roerer proces-global tilstand", og hver
//! threads-testfil tager den. Sikkerheden er dermed en egenskab ved hjaelperen
//! i stedet for en vane hos forfatteren — det er forskellen paa at holde og
//! paa at holde indtil nogen glemmer det.
//!
//! NAAR NOGEN ALLIGEVEL GLEMMER DET, er `scripts/data-dir-guard.mjs` det der
//! bliver roedt — og at den faktisk BLIVER roedt, er bevist og ikke paastaaet:
//! `tests/data_dir_guard_negative.rs` begaar fejlen med vilje og dokumenterer
//! den eksakte kommandosekvens der faelder vagten. Den fil er gatet BAADE af
//! `required-features = ["live-data-probe"]` og af `#[ignore]`, fordi den
//! forurener den levende installation, og den maa aldrig tage `serial()` —
//! saa ville den arve sandkassen og bevise ingenting.
//!
//! ÉN STI SANDKASSES IKKE AF `serial()`, og den er vaerd at kende foer man
//! skriver en test der roerer den: `voice_capture::default_capture_path()`
//! oploeser `%LOCALAPPDATA%\Talminal\voice-eval\` direkte fra `LOCALAPPDATA`
//! og laeser hverken `TALMINAL_HOME` eller `TALMINAL_GLOBAL_HOME`. Vagten
//! FANGER den (mappen er ikke undtaget), men raadet "tag serial()" hjaelper
//! ikke dér — brug `reset_capture_at`/`append_capture_at` med eksplicit sti.

// Modulet kompileres ind i HVER testbinary der siger `mod common;`, og hver
// binary bruger sin egen delmaengde — pty-suiterne roerer aldrig `serial()`,
// threads-suiterne roerer aldrig ANSI-stripperen. Uden dette ville hver binary
// faa dead_code-advarsler for alt den ikke bruger, og CI koerer clippy med
// `-D warnings`. Derfor ét allow her frem for et pr. funktion.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

fn serial_lock() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

/// Sandkassens rod: `target/tmp/<testbinary>/`. `CARGO_TARGET_TMPDIR` er kun
/// sat naar cargo kompilerer et integrations-test- eller bench-target, saa
/// stien findes ikke i biblioteket og kan ikke ved et uheld ramme appen. Den
/// ligger under `target/`, saa `cargo clean` rydder den, den er gitignoreret,
/// og den lækker ikke temp-mapper i OS'et som en `TempDir` i en `OnceLock`
/// ville. Pr. testbinary — saa to binaries der koerer parallelt ikke deler
/// mappe, hvilket de faktisk gjorde da de begge pegede paa den rigtige.
fn sandbox_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(env!("CARGO_CRATE_NAME"));
        std::fs::create_dir_all(&root).expect("sandbox data home");
        root
    })
}

/// Global-rod for sandkassen — søskende til `sandbox_root()`, aldrig samme
/// mappe. `global_base()` (project.rs) er roden hvor `settings.json`,
/// `last_project` og `active_workspace.json` bor; `talminal_base()` er
/// projekt-state-dir'en ét niveau nede (`sandbox_root()` ovenfor). De skal
/// være adskilte i testen, ellers ville en test der skriver workspace.json
/// kunne ramme settings.json.
///
/// Samme begrundelse som `sandbox_root()`: under `CARGO_TARGET_TMPDIR`, ikke
/// i OS'ets temp-mappe, saa `cargo clean` rydder den og den ikke lækker en
/// temp-mappe pr. testbinary pr. koersel.
fn global_sandbox_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(concat!(env!("CARGO_CRATE_NAME"), "-global"));
        std::fs::create_dir_all(&root).expect("sandbox global home");
        root
    })
}

/// Tages som FOERSTE linje i hver test der roerer proces-global tilstand.
/// Poison ignoreres: en panicking test maa ikke laase resten af filen ud.
///
/// Sætter OGSAA `TALMINAL_HOME` og `TALMINAL_GLOBAL_HOME` til sandkassen —
/// med laasen i haanden, saa skrivningen ikke kapper benene under en
/// samtidig test (det var praecis T12-fundets fælde). At den gøres ved HVERT
/// kald og ikke kun én gang er bevidst: en tidligere test i samme binary kan
/// have peget env'en paa sin egen `temp_home()`, som er slettet igen da dens
/// `TempDir` blev droppet.
pub fn serial() -> MutexGuard<'static, ()> {
    let guard = serial_lock().lock().unwrap_or_else(|p| p.into_inner());
    std::env::set_var("TALMINAL_HOME", sandbox_root());
    std::env::set_var("TALMINAL_GLOBAL_HOME", global_sandbox_root());
    guard
}

/// Peger TALMINAL_HOME paa en frisk temp-mappe. Holdes levende af kalderen;
/// naar TempDir droppes, ryddes filerne.
///
/// Bruges kun af de tests der har brug for en GARANTERET TOM mappe (fx
/// arkiv-scanninger der taeller filer). Alle andre er daekket af `serial()`
/// og behoever den ikke.
pub fn temp_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::env::set_var("TALMINAL_HOME", dir.path());
    dir
}

// ---------------------------------------------------------------------------
// ConPTY-harnessen (delt af pty_host.rs og profiles.rs)
//
// De to suiter havde hver sin BYTE-IDENTISKE kopi af `strip_ansi` og dens
// foelgesvende. Begrundelsen stod skrevet i profiles.rs — "test-binaries kan
// ikke dele kode uden tests/common/, som er uden for lease" — og den er ikke
// sand laengere: denne fil findes og inkluderes allerede af 26 testfiler.
//
// Det er ikke en kosmetisk dublering. `strip_ansi` er den maalestok begge
// PTY-suiter bruger til at afgoere om "outputtet indeholdt X"; en rettelse i
// den ene kopis OSC-/DCS-grene ville efterlade den anden suite blind.
// ---------------------------------------------------------------------------

pub fn cmd_exe() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!(r"{root}\System32\cmd.exe")
}

// ---------------------------------------------------------------------------
// Raa MCP-HTTP-klient (delt af mcp_server.rs og mcp_threads.rs)
//
// Request-formen ER serverens accepterede flade: `mcp.rs` dokumenterer at
// `Content-Length`-haandteringen er BEVIDST striks. Den strikshed blev
// tidligere asserteret mod to uafhaengigt vedligeholdte request-byggere.
// ---------------------------------------------------------------------------

/// Sender én `POST /mcp` og giver det RAA svar (headers inkl.), saa en test
/// kan paastaa noget om statuslinjen. `session` og `bearer` saettes uafhaengigt,
/// saa forrangsreglerne mellem de to identitets-kanaler kan proeves.
pub fn mcp_post_raw(port: u16, body: &str, session: Option<&str>, bearer: Option<&str>) -> String {
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let session_header = session
        .map(|s| format!("x-talminal-session: {s}\r\n"))
        .unwrap_or_default();
    let auth_header = bearer
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\n{session_header}{auth_header}\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).expect("write");
    let mut out = String::new();
    stream.read_to_string(&mut out).expect("read");
    out
}

/// Deler et raat HTTP-svar ved header/body-graensen og parser bodyen som JSON.
pub fn body_json(raw: &str) -> serde_json::Value {
    let (_headers, body) = raw
        .split_once("\r\n\r\n")
        .expect("http response has a header/body separator");
    serde_json::from_str(body).expect("body is valid json")
}

/// Er DENNE proces den navngivne worker-child? `TALMINAL_WORKER` er
/// foraeldre↔barn-kontrakten for de tests der skal koere i en frisk proces
/// (proces-global tilstand kan ikke nulstilles i traaden). Navnet er en wire —
/// derfor ét sted, ikke tre.
pub fn is_worker(name: &str) -> bool {
    std::env::var("TALMINAL_WORKER").as_deref() == Ok(name)
}

/// Styrbart ur til traad-dispatchens tests. Laa i tre byte-identiske kopier
/// (`threads_dispatch`, `threads_heartbeat`, `threads_sweep`) — en aendring i
/// `dispatch::Clock` braekker ellers tre haandskrevne doubler.
pub struct FakeClock(pub Arc<Mutex<u64>>);

impl talminal_canvas_lib::threads::dispatch::Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        *self.0.lock().unwrap()
    }
}

/// ANSI-stripper fra spike-harnessen (kanonisk moenster) — sentinel-soegning
/// maa ikke forstyrres af escape-sekvenser i ConPTY-outputtet.
pub fn strip_ansi(bytes: &[u8]) -> String {
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

pub fn stripped(buf: &Arc<Mutex<Vec<u8>>>) -> String {
    strip_ansi(&buf.lock().unwrap())
}

pub fn wait_for(buf: &Arc<Mutex<Vec<u8>>>, needle: &str, timeout: Duration) -> bool {
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

/// Venter paa child-exit og giver exit-koden. De to suiter havde hver sin
/// variant — `Option<u32>` og en `bool` — af den samme loekke; den rigere form
/// er valgt, saa en test der VIL se koden ikke skal skrive loekken igen.
pub fn wait_exit(host: &talminal_canvas_lib::pty::PtyHost, timeout: Duration) -> Option<u32> {
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
