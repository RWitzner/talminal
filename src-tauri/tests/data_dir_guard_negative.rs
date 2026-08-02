//! Den GLEMSOMME test: beviset for at `scripts/data-dir-guard.mjs` faktisk
//! faelder noget.
//!
//! `tests/data_sandbox.rs` beviser at sandkassen VIRKER naar man tager
//! `common::serial()`. Den beviser intet om hvad der sker naar man glemmer den
//! — og det er praecis den fejl vagten findes for. En vagt der aldrig har
//! faeldet noget er en paastand, ikke et vaern: en groen suite er lige saa
//! groen naar tjekket er brudt som naar der ikke er noget at fange.
//!
//! Derfor ligger her tre tests der med VILJE begaar fejlen.
//!
//! DE ER GATET TO GANGE, og begge gates er noedvendige. Filen bygges kun med
//! `--features live-data-probe` (`required-features` i Cargo.toml, samme
//! moenster som `tests/keyring_smoke.rs`), OG hver test er `#[ignore]`d.
//! `#[ignore]` alene var ikke nok: `cargo test -- --ignored` er en helt
//! almindelig inkantation — `tests/perf_pty_output.rs` dokumenterer den selv —
//! og den ville skrive sonderne permanent ind i ejerens LEVENDE installation.
//! Sker det midt i et ritual mellem `snapshot` og `verify`, bliver `verify`
//! roed uden at kunne skelnes fra en aegte regression. En test der findes for
//! at forhindre utilsigtet skrivning i den levende datamappe maa ikke selv
//! vaere den nemmeste vej til at goere det.
//!
//! HVORFOR EGEN FIL: `common::serial()` saetter `TALMINAL_HOME` og
//! `TALMINAL_GLOBAL_HOME` som PROCES-globale env-vars. I en fil hvor en anden
//! test tager `serial()`, ville disse to arve sandkassen og bevise ingenting —
//! de ville skrive i `target/tmp/` og vagten ville med rette tie. En frisk
//! testbinaer hvor INGEN test tager `serial()` har env'en uroert, og det er
//! netop situationen "en ny testfil hvis forfatter ikke kendte reglen".
//! Tilfoejer nogen en `serial()`-test i denne fil, holder beviset op med at
//! vaere et bevis. Laeg den et andet sted.
//!
//! De to tests fjerner heller ikke selv env-vars: saa var de fjendtlige frem
//! for glemsomme, og de ville ikke laengere reproducere fejlklassen.
//!
//! # Saadan faelder man vagten med vilje (den EKSAKTE sekvens)
//!
//! Fra repo-roden. FORUDSAETNING: en shell hvor `TALMINAL_HOME` og
//! `TALMINAL_GLOBAL_HOME` er TOMME. Er de sat — og det er de i enhver
//! Talminal-kort-terminal, fordi `main.rs` peger `TALMINAL_HOME` paa
//! `projects\<slug>` ved opstart — lander sonderne et andet sted end
//! oprydningen nedenfor peger. `--nocapture` er med, fordi libtest ellers
//! opsluger stdout for BESTAAEDE tests, og disse tests bestaar; det er
//! `verify` der skal fejle. Uden det ser du aldrig stien de printer.
//!
//! Hver linje staar ubrudt, saa sekvensen kan klippes direkte ind i baade
//! PowerShell og et CI-step. Kør ÉN sonde ad gangen med sit eget snapshot,
//! saa de to koerslers baselines ikke blandes.
//!
//! ```text
//! node scripts/data-dir-guard.mjs snapshot
//! cd src-tauri
//! cargo test --features live-data-probe --test data_dir_guard_negative -- --ignored --exact glemsom_test_uden_serial_skriver_i_traad_arkivet --nocapture
//! cd ..
//! node scripts/data-dir-guard.mjs verify      # SKAL fejle med exit 1
//! ```
//!
//! ```text
//! node scripts/data-dir-guard.mjs snapshot
//! cd src-tauri
//! cargo test --features live-data-probe --test data_dir_guard_negative -- --ignored --exact glemsom_test_uden_serial_skriver_i_hud_mappen --nocapture
//! cd ..
//! node scripts/data-dir-guard.mjs verify      # SKAL fejle med exit 1
//! ```
//!
//! `verify` skal skrive "testsuiten skrev i den RIGTIGE datamappe" og
//! afslutte med exit 1. Gør den det ikke, er vagten gaaet i stykker — ikke
//! testen. Den anden sekvens beviser desuden at `EXTERNAL_WRITERS`-
//! undtagelsen ikke er blevet saa bred at en glemsom test kan gemme sig i den.
//!
//! Den TREDJE test, `glemsom_sonde_i_hud_context_giver_kun_en_note`, er
//! modstykket: den dokumenterer vagtens ene accepterede blindhed og skal give
//! `verify` exit **0**. Den koeres paa samme maade, og et exit 1 dér betyder at
//! nogen har indsnaevret `hud\context\`-moensteret — hvilket kan vaere rigtigt,
//! men saa skal tap'ens egne context-filer stadig kunne skrives uden falsk roed.
//!
//! OPRYDNING:
//!
//! ```text
//! Remove-Item "$env:LOCALAPPDATA\Talminal\threads\data-dir-guard-negative-probe.jsonl"
//! Remove-Item "$env:LOCALAPPDATA\Talminal\hud\data-dir-guard-negative-probe.json"
//! Remove-Item "$env:LOCALAPPDATA\Talminal\hud\context\data-dir-guard-negative-probe.json"
//! rmdir "$env:LOCALAPPDATA\Talminal\threads"
//! rmdir "$env:LOCALAPPDATA\Talminal\hud\context"
//! node scripts/data-dir-guard.mjs snapshot    # frisk baseline efter oprydning
//! ```
//!
//! De to `rmdir`-linjer er der fordi sonderne kalder `create_dir_all` og
//! altsaa OPRETTER mapperne hvis de ikke fandtes. **Vagten indekserer kun
//! FILER**, saa en efterladt tom mappe bliver aldrig roed og kan ligge for
//! evigt i en fremmeds installation. `rmdir` fejler hvis mappen ikke er tom —
//! praecis den sikkerhed man vil have her. Findes mappen i forvejen med
//! indhold, er fejlen den rigtige opfoersel: saa skal den blive.
//!
//! Ryd ALTID op bagefter — filerne ligger i en LEVENDE installation. Slet kun
//! `data-dir-guard-negative-*`; alt andet i mapperne er ejerens.

use std::path::{Path, PathBuf};

use talminal_canvas_lib::context_hud;
use talminal_canvas_lib::threads::archive;
use talminal_canvas_lib::usage_hud;

/// Faellesnavn for begge sonder. Praefikset er entydigt vores, saa oprydningen
/// kan vaere kirurgisk i en mappe hvor alt andet er ejerens data.
const PROBE_STEM: &str = "data-dir-guard-negative-probe";

/// Skriver sonden og fortaeller hvor den landede.
///
/// Selve skrivningen er `std::fs`, men STIEN kommer fra biblioteket
/// (`archive::threads_dir()` / `usage_hud::snapshot_path()`). Det er den vej
/// rundt der er pointen: en test der selv samlede
/// `%LOCALAPPDATA%\Talminal\...` ville bevise at MAN KAN skrive der, ikke at
/// vagten fanger den fejlklasse produktet faktisk har. Sandkasses stien en dag
/// ved roden, holder disse tests op med at ramme installationen — og det er
/// den rigtige opfoersel.
///
/// Vi kalder ikke `archive::append_lines()`, selvom det ville vaere endnu et
/// lag bibliotek: den kraever et traad-id paa formen `t<cifre>` og ville altsaa
/// skrive `t<n>.jsonl` — et navn i ejerens EGET id-rum, som en oprydning ikke
/// kan skelne fra en aegte traad. Stien er bibliotekets; navnet skal vaere
/// vores.
fn skriv_sonde(dir: &Path, filnavn: &str) -> PathBuf {
    assert!(
        !dir.starts_with(Path::new(env!("CARGO_TARGET_TMPDIR"))),
        "sonden landede i sandkassen ({}) — saa er env'en allerede sat, og \
         testen beviser intet. Tager en anden test i DENNE fil common::serial()?",
        dir.display()
    );
    std::fs::create_dir_all(dir).expect("opret maalmappen");
    let path = dir.join(filnavn);
    std::fs::write(&path, b"data-dir-guard negative probe\n").expect("skriv sonden");
    println!("glemsom sonde skrevet: {}", path.display());
    path
}

/// Fejlklassen som vagten blev bygget til: en test uden `common::serial()`
/// opløser `talminal_base()/threads` til den levende installation og skriver
/// der. Historisk kostede det ejeren +32,8 KB i `threads\t1.jsonl` pr. koersel
/// (se `tests/common/mod.rs`).
///
/// `threads/` er IKKE i vagtens `EXTERNAL_WRITERS`, saa den skal give HAARD
/// fejl: `verify` exit 1.
#[test]
#[ignore]
fn glemsom_test_uden_serial_skriver_i_traad_arkivet() {
    // Med vilje INGEN common::serial() her. Det er hele testen.
    skriv_sonde(&archive::threads_dir(), &format!("{PROBE_STEM}.jsonl"));
}

/// Samme fejl, men i den ene undermappe vagten behandler mildt.
///
/// `EXTERNAL_WRITERS` degraderer aendringer i `hud\`, `ingress\` og
/// `presence\` fra fejl til note, fordi statusline-tap'en og Python-
/// controlleren skriver der mens ritualet koerer. Undtagelsen maa ikke vaere
/// saa bred at en glemsom test kan gemme sig i den: `usage_hud::snapshot_path()`
/// opløser `global_base()\hud\usage.json`, og en test der ville lave en fixtur
/// til `read_snapshot()` lander praecis her.
///
/// Efter T3 skelner vagten mellem tap'ens moenster (den SAMME faste fil
/// skrevet om igen) og en ny fil med et ukendt navn. Derfor skal ogsaa denne
/// give exit 1.
#[test]
#[ignore]
fn glemsom_test_uden_serial_skriver_i_hud_mappen() {
    // Med vilje INGEN common::serial() her heller.
    let hud = usage_hud::snapshot_path()
        .parent()
        .expect("usage.json har en foraeldremappe")
        .to_path_buf();
    skriv_sonde(&hud, &format!("{PROBE_STEM}.json"));
}

/// Vagtens ENE accepterede blindhed — pinnet, saa den ikke kan vokse tavst.
///
/// `hud\context\<navn>.json` er tap'ens eget navnerum, og `contextFileKey` i
/// `statusline-tap/lib.mjs` tillader hele `[A-Za-z0-9._-]+`. Et fixtur-navn kan
/// derfor ikke skelnes fra et kortnavn, og vagten maa ikke faelde paa dem —
/// ellers giver et helt normalt kort falsk roed midt i ritualet.
///
/// Prisen er at en glemsom test der lander praecis dér, kun bliver en NOTE.
/// `context_hud::context_dir()` er `pub` og ét kald vaek, saa det er ikke en
/// teoretisk vej. Vi accepterer den bevidst — men her, hvor accepten er MAALT
/// i stedet for paastaaet.
///
/// **Forventet udfald: `verify` exit 0 med en note-linje.** Giver den exit 1,
/// er `hud\context\`-moensteret blevet snaevrere; det kan vaere den rigtige
/// beslutning, men saa skal tap'ens egne context-filer stadig kunne skrives
/// uden falsk roed, og `scripts/data-dir-guard.test.mjs` skal opdateres.
#[test]
#[ignore]
fn glemsom_sonde_i_hud_context_giver_kun_en_note() {
    // Med vilje INGEN common::serial() her heller.
    skriv_sonde(&context_hud::context_dir(), &format!("{PROBE_STEM}.json"));
}
