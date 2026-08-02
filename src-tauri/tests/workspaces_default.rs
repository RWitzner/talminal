//! T12 — default-workspacets synlighed og `last_project`-hintets graenser.
//!
//! Tre af testene kommer ordret fra planens Step 1; de fire oevrige daekker de
//! krav planen beskriver i proesa, men ikke tester: at en eksplicit `.hidden`
//! slaar en eksplicit `.unhidden`, at `resolve_startup_home()` springer et hint
//! til et SKJULT projekt over (og kun det), og at default-workspacets no-op i
//! `write_last_project_for_active` ikke afhaenger af en manglende `project.json`.
//!
//! **Datamappe-faren:** `resolve_startup_home()` opløser `global_base()` paa
//! KALDSTIDSPUNKTET. Uden `TALMINAL_GLOBAL_HOME` er det ejerens levende
//! `%LOCALAPPDATA%\Talminal`, og testen ville skrive `projects\default` +
//! `last_project` dér. Derfor sandkasser de to `resolve_startup_home`-tests
//! env'en gennem `EnvSandkasse` — som ogsaa FJERNER `TALMINAL_HOME`, fordi
//! `resolve_startup_home` tjekker den foerst (instance.rs:141-145) og ellers
//! ville returnere foer hint-grenen overhovedet blev naaet.

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use talminal_canvas_lib::instance::{self, resolve_startup_home};
use talminal_canvas_lib::project::{
    self, normalize_path, project_state_dir, write_last_project, write_project_meta, ProjectMeta,
};
use talminal_canvas_lib::workspaces::listing;

fn skriv(base: &Path, slug: &str, navn: &str) {
    let dir = base.join("projects").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.json"),
        format!(
            r#"{{"root":"C:\\r\\{navn}","name":"{navn}","added_at":"2026-07-01T00:00:00.000Z"}}"#
        ),
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Env-sandkasse (F13). Forlaeg: tests/instance.rs:22-26.
// ---------------------------------------------------------------------------

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Holder env-laasen og gendanner begge variable i `Drop`, saa en panicking
/// assert ikke efterlader `TALMINAL_GLOBAL_HOME` pegende paa en slettet
/// tempdir for resten af binaryen.
struct EnvSandkasse {
    forrige_global: Option<OsString>,
    forrige_home: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl EnvSandkasse {
    fn ny(global: &Path) -> Self {
        // Poison ignoreres: en panicking test maa ikke laase resten af filen ud.
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let forrige_global = std::env::var_os("TALMINAL_GLOBAL_HOME");
        let forrige_home = std::env::var_os("TALMINAL_HOME");
        std::env::set_var("TALMINAL_GLOBAL_HOME", global);
        std::env::remove_var("TALMINAL_HOME");
        Self {
            forrige_global,
            forrige_home,
            _guard: guard,
        }
    }
}

impl Drop for EnvSandkasse {
    fn drop(&mut self) {
        gendan("TALMINAL_GLOBAL_HOME", self.forrige_global.take());
        gendan("TALMINAL_HOME", self.forrige_home.take());
    }
}

fn gendan(key: &str, vaerdi: Option<OsString>) {
    match vaerdi {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

/// Registrerer et rigtigt projekt i sandkassen og peger `last_project` paa det.
/// Returnerer projektets state-dir (`<global>/projects/<slug>`).
fn registrer_og_saet_hint(root: &Path, navn: &str) -> std::path::PathBuf {
    let root = normalize_path(root).unwrap();
    write_last_project(&root).unwrap();
    let state = project_state_dir(&root).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    write_project_meta(
        &state,
        &ProjectMeta {
            root,
            name: navn.into(),
            added_at: None,
        },
    )
    .unwrap();
    state
}

// ---------------------------------------------------------------------------
// 1-3: planens Step 1, ordret.
// ---------------------------------------------------------------------------

#[test]
fn default_er_synligt_naar_det_staar_alene() {
    let base = tempfile::tempdir().unwrap();
    skriv(base.path(), "default", "default");
    let liste = listing::list(base.path());
    assert_eq!(liste.len(), 1);
    assert!(
        !liste[0].hidden,
        "førstegangs-brugeren skal kunne se sit ene workspace"
    );
}

#[test]
fn default_skjules_automatisk_naar_et_rigtigt_projekt_findes() {
    let base = tempfile::tempdir().unwrap();
    skriv(base.path(), "default", "default");
    skriv(base.path(), "p-1111", "demo");

    let liste = listing::list(base.path());
    let d = liste.iter().find(|e| e.slug == "default").unwrap();
    assert!(
        d.hidden,
        "default må ikke blive liggende når brugeren har rigtige projekter"
    );
    assert!(!liste.iter().find(|e| e.slug == "p-1111").unwrap().hidden);
}

#[test]
fn en_eksplicit_unhide_af_default_respekteres() {
    let base = tempfile::tempdir().unwrap();
    skriv(base.path(), "default", "default");
    skriv(base.path(), "p-1111", "demo");
    // Brugeren har hentet default frem via "Vis skjulte" → sidecar-fraværet er
    // ikke nok; en eksplicit markør vinder over den automatiske regel.
    std::fs::write(
        base.path()
            .join("projects")
            .join("default")
            .join(".unhidden"),
        b"",
    )
    .unwrap();

    let liste = listing::list(base.path());
    assert!(!liste.iter().find(|e| e.slug == "default").unwrap().hidden);
}

// ---------------------------------------------------------------------------
// 4: F4 — .hidden slaar .unhidden. De to filer KAN sameksistere, fordi
// listing::hide (listing.rs:86-90) ikke sletter .unhidden, og listing::unhide
// (:99-103) skriver .unhidden for ethvert slug. Reglen skal derfor skrives som
// `entry.hidden = entry.hidden || !eksplicit_frem` — ikke som en ren tildeling.
// ---------------------------------------------------------------------------

#[test]
fn en_eksplicit_hide_af_default_vinder_over_unhidden() {
    let base = tempfile::tempdir().unwrap();
    skriv(base.path(), "default", "default");
    skriv(base.path(), "p-1111", "demo");
    let dir = base.path().join("projects").join("default");
    // Rækkefølgen paa disken: brugeren hentede default frem, og skjulte den saa
    // igen. `hide` sletter ikke markøren, saa begge filer ligger der bagefter.
    std::fs::write(dir.join(".unhidden"), b"").unwrap();
    std::fs::write(dir.join(".hidden"), b"").unwrap();

    let liste = listing::list(base.path());
    assert!(
        liste.iter().find(|e| e.slug == "default").unwrap().hidden,
        "en eksplicit .hidden er brugerens seneste ord — .unhidden maa ikke overskrive den"
    );
}

// ---------------------------------------------------------------------------
// 5-6: .hidden-gaten i resolve_startup_home. Test 6 falsificerer test 5: en
// gate der sprang ALLE hints over ville bestaa test 5 og goere `last_project`
// betydningsloest.
// ---------------------------------------------------------------------------

#[test]
fn resolve_startup_home_springer_et_skjult_hint_over() {
    let global = tempfile::tempdir().unwrap();
    let _env = EnvSandkasse::ny(global.path());

    let root_tmp = tempfile::tempdir().unwrap();
    let state = registrer_og_saet_hint(root_tmp.path(), "fjernet-fra-listen");
    // "Fjern fra listen" = listing::hide → `.hidden` i projektets state-dir.
    std::fs::write(state.join(".hidden"), b"").unwrap();

    let got = resolve_startup_home().expect("et skjult hint er ikke korruption og maa ikke fejle");
    assert_eq!(
        got,
        global.path().join("projects").join("default"),
        "et hint til et SKJULT projekt maa ikke genaabne det — opstarten skal falde til default"
    );
}

#[test]
#[allow(non_snake_case)] // plan-mandated navn (§4-tabellens test 6)
fn resolve_startup_home_aabner_et_IKKE_skjult_hint() {
    let global = tempfile::tempdir().unwrap();
    let _env = EnvSandkasse::ny(global.path());

    let root_tmp = tempfile::tempdir().unwrap();
    let state = registrer_og_saet_hint(root_tmp.path(), "helt-normalt");
    assert!(
        !state.join(".hidden").exists(),
        "forudsaetning: posten er IKKE skjult"
    );

    let got = resolve_startup_home().expect("et normalt hint skal kunne aabnes");
    assert_eq!(
        got, state,
        "gaten maa kun ramme skjulte hints — ellers starter appen altid i default"
    );
}

// ---------------------------------------------------------------------------
// 7: F1 — default-workspacets state-dir (projects/default) er IKKE koblet til
// dets root (brugerens hjemmemappe). Skrives hintet, resolver naeste opstart
// project_state_dir(home) = projects/<brugernavn>-<hex8>: en tom tvillingemappe
// hvor brugerens kort, tråde og settings ikke findes.
// ---------------------------------------------------------------------------

#[test]
fn default_workspacet_skriver_aldrig_last_project() {
    let base = tempfile::tempdir().unwrap();
    // Praecis den situation kaldstederne i main.rs staar i: filen FINDES, den er
    // læsbar, og dens root er hjemmemappen. No-op'en maa ikke afhaenge af at
    // project.json mangler — saa var den kun tilfaeldigvis rigtig.
    let hjem = instance::user_home_dir().unwrap();
    let state = base.path().join("projects").join("default");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("project.json"),
        format!(
            r#"{{"root":"{}","name":"default","added_at":null}}"#,
            hjem.display().to_string().replace('\\', "\\\\")
        ),
    )
    .unwrap();
    assert!(
        project::read_project_meta(&state).unwrap().is_some(),
        "forudsaetning: default-workspacets project.json er læsbar"
    );

    project::write_last_project_for_active(base.path(), "default").unwrap();

    assert!(
        !base.path().join("last_project").exists(),
        "default maa ALDRIG skrive et last_project-hint — naeste opstart ville lande i en tom tvillingemappe"
    );
}
