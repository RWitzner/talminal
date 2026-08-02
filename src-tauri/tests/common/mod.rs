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

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

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
#[allow(dead_code)]
pub fn temp_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::env::set_var("TALMINAL_HOME", dir.path());
    dir
}
