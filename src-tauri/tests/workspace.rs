// Task 6 — workspace.json-persistens + cards.toml-engangsimport (lib-cratet).
//
// Testcases (planens (a)-(e)):
//   (a) roundtrip save/load byte-stabil                        — path-niveau, direkte
//   (b) crash mellem tmp-write og rename -> gammel fil intakt  — path-niveau, direkte
//   (c) engangsimport: 2 workers + [master] -> card-1/card-2   — worker-proces
//   (d) eksisterende workspace.json + cards.toml -> toml ignoreres — worker-proces
//   (e) next_card_number overlever restart (ingen nummer-genbrug) — 2 worker-processer
//
// WORKER-PROCES-MOENSTRET: registryet er en global singleton og TALMINAL_HOME
// er proces-global env — (c)/(d)/(e) kraever derfor FRISKE processer for at
// kunne asserte absolutte numre (card-1/card-2) og restart-semantik. Driver-
// testene spawner test-binaren selv (current_exe) med `--exact <worker>` og
// TALMINAL_WORKER som markoer; uden markoeren er worker-testene no-ops.
// Foraeldre-processen kalder ALDRIG startup_load/registry — (a)/(b) er rene
// fil-tests og parallel-robuste.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use talminal_canvas_lib::cards::CardConfig;
use talminal_canvas_lib::registry;

mod common;
use common::is_worker;
use talminal_canvas_lib::workspace::{
    self, load_settings, load_workspace_file, save_workspace_file, settings_path, Settings,
    SettingsInput, Viewport, WorkspaceCard, WorkspaceFile, DEFAULT_EXIT_TYPE_MODE_HOTKEY,
    DEFAULT_PTT_HOTKEY,
};

// ---------- hjaelpere ----------

fn sample_file() -> WorkspaceFile {
    WorkspaceFile {
        schema_version: 1,
        next_card_number: 4,
        viewport: Viewport {
            x: -120.5,
            y: 33.25,
            zoom: 0.75,
        },
        cards: vec![
            WorkspaceCard {
                number: 1,
                name: "card-1".to_string(),
                cwd: "C:/code/proj-a".to_string(),
                profile: "claude".to_string(),
                command: None,
                x: 10.0,
                y: 20.0,
                w: 960.0,
                h: 640.0,
                last_active_at: Some("2026-07-17T10:00:00.000Z".to_string()),
            },
            WorkspaceCard {
                number: 3,
                name: "card-3".to_string(),
                cwd: "C:/code/proj-b".to_string(),
                profile: "claude".to_string(),
                command: Some("cmd /c echo hi".to_string()),
                x: 58.0,
                y: 60.0,
                w: 800.0,
                h: 500.0,
                last_active_at: None,
            },
        ],
    }
}

fn settings_input(
    ptt: &str,
    exit: &str,
    engine: &str,
    wallpaper: &str,
    agent: &str,
) -> SettingsInput {
    SettingsInput {
        ptt_hotkey: ptt.to_string(),
        exit_type_mode_hotkey: exit.to_string(),
        voice_engine: engine.to_string(),
        wallpaper: wallpaper.to_string(),
        default_agent: agent.to_string(),
        stt_provider: "openai".to_string(),
        routing_provider: "vercel".to_string(),
        // Dikterings-felterne holdes paa deres defaults her, saa de mange
        // eksisterende kaldere bevarer signaturen. De tests der handler OM
        // dikteringen saetter dem selv paa den returnerede struct.
        dictation_hotkey: workspace::DEFAULT_DICTATION_HOTKEY.to_string(),
        dictation_submit: false,
        // TOM og ikke standardlisten: helperen skal ikke smugle en default ind
        // i tests der ikke handler om keywords. De tests der GOER, saetter den
        // selv paa den returnerede struct.
        stt_keywords: Vec::new(),
    }
}

/// Spawner test-binaren selv med `--exact <worker>` i en FRISK proces med
/// TALMINAL_HOME sat — registry-taeller og workspace-singleton starter forfra.
fn run_worker(worker: &str, home: &Path) {
    run_worker_with_global(worker, home, None);
}

/// Som `run_worker`, men kan ogsaa saette TALMINAL_GLOBAL_HOME (B-light T4
/// settings.json bor globalt — tests maa ikke skrive i rigtige LOCALAPPDATA).
fn run_worker_with_global(worker: &str, home: &Path, global: Option<&Path>) {
    let exe = std::env::current_exe().expect("current_exe");
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--exact")
        .arg(worker)
        .arg("--test-threads=1")
        .arg("--nocapture")
        .env("TALMINAL_WORKER", worker)
        .env("TALMINAL_HOME", home);
    if let Some(g) = global {
        cmd.env("TALMINAL_GLOBAL_HOME", g);
    }
    let out = cmd.output().expect("spawn worker test process");
    assert!(
        out.status.success(),
        "worker '{worker}' failed\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn home_from_env() -> PathBuf {
    PathBuf::from(std::env::var("TALMINAL_HOME").expect("worker needs TALMINAL_HOME"))
}

fn read_json(path: &Path) -> serde_json::Value {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Skriver en cards.toml med 2 workers + [master] (master har command, saa
/// filen ogsaa er gyldig under `--features supervision`). Returnerer de to
/// worker-cwd'er (reelle dirs — create_card validerer is_dir).
fn write_cards_toml(home: &Path) -> (PathBuf, PathBuf) {
    let dir_a = home.join("proj-alpha");
    let dir_b = home.join("proj-beta");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();
    let fwd = |p: &Path| p.display().to_string().replace('\\', "/");
    let toml = format!(
        r#"[master]
cwd = "{home_fwd}"
command = ["cmd", "/c", "echo", "feed"]

[[card]]
name = "alpha"
cwd = "{a}"

[[card]]
name = "beta"
cwd = "{b}"
command = ["cmd", "/c", "ping", "-t", "127.0.0.1"]
resume_command = ["cmd", "/c", "ping", "-t", "127.0.0.1"]
"#,
        home_fwd = fwd(home),
        a = fwd(&dir_a),
        b = fwd(&dir_b),
    );
    std::fs::write(home.join("cards.toml"), toml).unwrap();
    (dir_a, dir_b)
}

// ---------- (a) roundtrip save/load byte-stabil ----------

#[test]
fn roundtrip_save_load_is_byte_stable() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("workspace.json");
    let p2 = dir.path().join("workspace2.json");

    let original = sample_file();
    save_workspace_file(&p1, &original).expect("first save");
    let bytes1 = std::fs::read(&p1).unwrap();

    let loaded = load_workspace_file(&p1)
        .expect("load must succeed")
        .expect("file exists");
    assert_eq!(
        loaded, original,
        "load skal gengive strukturen felt-for-felt"
    );

    save_workspace_file(&p2, &loaded).expect("second save");
    let bytes2 = std::fs::read(&p2).unwrap();
    assert_eq!(
        bytes1, bytes2,
        "save(load(save(x))) skal vaere byte-identisk (deterministisk serialisering)"
    );
}

// ---------- (b) atomicitet: crash mellem tmp-write og rename ----------

#[test]
fn crash_between_tmp_write_and_rename_leaves_old_file_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");

    let original = sample_file();
    save_workspace_file(&path, &original).expect("initial save");
    let good_bytes = std::fs::read(&path).unwrap();

    // Simuleret crash: en efterladt, halvskrevet tmp-fil ved siden af den
    // gamle fil (writeren doede mellem tmp-write og rename).
    let tmp = dir.path().join("workspace.json.tmp");
    std::fs::write(&tmp, b"{\"schema_version\":1,\"next_card_nu").unwrap();

    let loaded = load_workspace_file(&path)
        .expect("gammel fil skal kunne laeses uden parse-fejl")
        .expect("file exists");
    assert_eq!(
        loaded, original,
        "den gamle fil er intakt trods efterladt tmp"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        good_bytes,
        "load maa ikke roere den gamle fil"
    );

    // Naeste save skal overleve den efterladte tmp (replace, ikke fejl).
    let mut next = original.clone();
    next.next_card_number = 9;
    save_workspace_file(&path, &next).expect("save over efterladt tmp");
    let reloaded = load_workspace_file(&path).unwrap().unwrap();
    assert_eq!(reloaded.next_card_number, 9);
}

// ---------- (c) frisk start: cards.toml laeses ALDRIG (ejer-amendment 2026-07-19) ----------

#[test]
fn first_load_imports_cards_toml_renames_and_skips_master() {
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path();
    write_cards_toml(home); // ligger klar — men maa ALDRIG laeses

    // Isoleret global-dir: uden den laeser workeren den RIGTIGE globale
    // settings.json, og `settings == Settings::default()`-assertet maaler
    // udviklerens maskine i stedet for frisk-start-semantikken (opdaget ved
    // T11-flippet, hvor default-aendringen afsloerede laekagen).
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global("worker_import_first_load", home, Some(global_dir.path()));

    // Foraeldre-side: workspace.json indeholder KUN workerens ene friske kort
    // — intet importeret fra cards.toml (tom-canvas-semantikken).
    let ws = read_json(&home.join("workspace.json"));
    assert_eq!(ws["schema_version"], 1);
    assert_eq!(ws["next_card_number"], 2, "ét frisk kort -> next=2: {ws}");
    let cards = ws["cards"].as_array().expect("cards array");
    assert_eq!(cards.len(), 1, "kun det friske kort — ingen import: {ws}");
    assert_eq!(cards[0]["number"], 1);
    assert_eq!(cards[0]["name"], "card-1");
    let names: Vec<&str> = cards.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(!names.contains(&"alpha") && !names.contains(&"beta") && !names.contains(&"master"));
}

#[test]
fn worker_import_first_load() {
    if !is_worker("worker_import_first_load") {
        return;
    }
    let home = home_from_env();

    let report = workspace::startup_load().expect("startup_load fresh");
    assert!(
        !report.cards_missing,
        "rapporten er konstant uden import-sti"
    );
    assert!(
        report.cards_error.is_none(),
        "rapporten er konstant uden import-sti"
    );

    // Tom start: intet i registryet, wire-formen er tom med default-viewport.
    assert!(
        registry::list_cards().is_empty(),
        "canvas starter ALTID tomt"
    );
    let via_cmd = workspace::get_workspace().expect("get_workspace fresh");
    assert!(via_cmd.cards.is_empty());
    assert_eq!(via_cmd.next_card_number, 1);
    assert_eq!(
        via_cmd.viewport,
        Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0
        }
    );
    assert_eq!(via_cmd.settings, Settings::default());

    // Ét frisk kort persisteres synkront — det er det ENESTE i filen.
    let cwd = home.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let info = workspace::create_card_persisted(
        cwd.display().to_string(),
        Some("claude".to_string()),
        None,
    )
    .expect("create fresh card");
    assert_eq!((info.number, info.name.as_str()), (1, "card-1"));
}

// ---------- (d) eksisterende workspace.json IGNORERES ved launch (ejer-amendment) ----------

#[test]
fn existing_workspace_json_wins_and_cards_toml_is_ignored() {
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path();
    write_cards_toml(home); // ligger klar — men maa ALDRIG laeses

    // Eksisterende workspace: ét kort card-7, next=8 (numre 1-6 er historik).
    let card_dir = home.join("proj-seven");
    std::fs::create_dir_all(&card_dir).unwrap();
    let existing = WorkspaceFile {
        schema_version: 1,
        next_card_number: 8,
        viewport: Viewport {
            x: 5.0,
            y: 6.0,
            zoom: 1.5,
        },
        cards: vec![WorkspaceCard {
            number: 7,
            name: "card-7".to_string(),
            cwd: card_dir.display().to_string(),
            profile: "claude".to_string(),
            command: None,
            x: 1.0,
            y: 2.0,
            w: 900.0,
            h: 600.0,
            last_active_at: None,
        }],
    };
    save_workspace_file(&home.join("workspace.json"), &existing).unwrap();

    run_worker("worker_load_ignores_toml", home);

    let ws = read_json(&home.join("workspace.json"));
    let cards = ws["cards"].as_array().unwrap();
    let names: Vec<&str> = cards.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(
        !names.contains(&"alpha") && !names.contains(&"beta"),
        "cards.toml maa aldrig laeses: {names:?}"
    );
    assert!(
        !names.contains(&"card-7"),
        "gamle kort maa ALDRIG overleve en launch (tom canvas hver gang): {names:?}"
    );
    assert_eq!(names, ["card-1"], "kun workerens friske kort: {names:?}");
    assert_eq!(ws["next_card_number"], 2, "taelleren starter forfra: {ws}");
    // Geometri-/viewport-persist fra workeren er landet i filen.
    let c1 = cards.iter().find(|c| c["name"] == "card-1").unwrap();
    assert_eq!(c1["x"], 11.0);
    assert_eq!(c1["y"], 22.0);
    assert_eq!(c1["w"], 333.0);
    assert_eq!(c1["h"], 444.0);
    assert_eq!(ws["viewport"]["zoom"], 1.25);
}

#[test]
fn worker_load_ignores_toml() {
    if !is_worker("worker_load_ignores_toml") {
        return;
    }
    let home = home_from_env();

    let report = workspace::startup_load().expect("startup_load fresh");
    assert!(
        !report.cards_missing && report.cards_error.is_none(),
        "rapporten er konstant"
    );

    // Tom start: den eksisterende fil (card-7, next=8, viewport 5/6/1.5)
    // ignoreres FULDSTAENDIGT — kort, taeller og viewport starter forfra.
    let ws = workspace::get_workspace().expect("get_workspace");
    assert!(ws.cards.is_empty(), "gamle kort genskabes aldrig: {ws:?}");
    assert_eq!(ws.next_card_number, 1, "taelleren starter forfra");
    assert_eq!(
        ws.viewport,
        Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0
        }
    );
    assert!(registry::list_cards().is_empty(), "registryet seedes ikke");

    // Foerste friske kort er card-1 — numre fra sidste session genbruges frit.
    let info = workspace::create_card_persisted(
        home.join("proj-seven").display().to_string(),
        Some("claude".to_string()),
        None,
    )
    .expect("create fresh");
    assert_eq!(info.number, 1, "frisk session -> nummer 1");
    assert_eq!(info.name, "card-1");

    // Geometri-aendring persisteres debounced (<=500 ms) — uaendret i-session.
    workspace::update_card_geometry("card-1".to_string(), 11.0, 22.0, 333.0, 444.0)
        .expect("update_card_geometry");
    let ws_path = home.join("workspace.json");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let v = read_json(&ws_path);
        let done = v["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "card-1" && c["x"] == 11.0 && c["h"] == 444.0);
        if done {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "debounced persist (<=500 ms) er ikke landet: {v}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // Viewport + persist_now (teardown-flushen).
    workspace::set_viewport(100.0, 200.0, 1.25).expect("set_viewport");
    workspace::persist_now().expect("persist_now");
    let v = read_json(&ws_path);
    assert_eq!(v["viewport"]["x"], 100.0);
    assert_eq!(v["viewport"]["zoom"], 1.25);

    // Ukendt kort er en beskrivende fejl.
    let err = workspace::update_card_geometry("card-999".to_string(), 0.0, 0.0, 1.0, 1.0)
        .expect_err("unknown card");
    assert!(err.contains("card-999"), "fik: {err}");
}

// ---------- (e) nummerering starter forfra ved hver launch (ejer-amendment) ----------

#[test]
fn next_card_number_survives_restart_no_number_reuse() {
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path();

    run_worker("worker_restart_a", home);

    // I-session-persist er uaendret: 3 oprettet, 2 lukket -> card-1, next=4.
    let ws = read_json(&home.join("workspace.json"));
    assert_eq!(
        ws["next_card_number"], 4,
        "3 oprettede kort -> next=4: {ws}"
    );
    let names: Vec<&str> = ws["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["card-1"],
        "card-2/card-3 blev lukket foer 'genstarten'"
    );

    run_worker("worker_restart_b", home);

    // NY launch = tom canvas: intet fra session A overlever, og taelleren
    // starter forfra (nummer-genbrug PAA TVAERS af launches er nu korrekt).
    let ws = read_json(&home.join("workspace.json"));
    assert_eq!(
        ws["next_card_number"], 2,
        "frisk session + ét kort -> next=2: {ws}"
    );
    let names: Vec<&str> = ws["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["card-1"],
        "session A's card-1 er VAEK; B's foerste kort er card-1"
    );
}

#[test]
fn worker_restart_a() {
    if !is_worker("worker_restart_a") {
        return;
    }
    let home = home_from_env();
    let report = workspace::startup_load().expect("startup_load empty first run");
    assert!(
        !report.cards_missing,
        "rapporten er konstant uden import-sti"
    );

    let cwd = home.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.display().to_string();
    let a =
        workspace::create_card_persisted(cwd.clone(), Some("claude".to_string()), None).unwrap();
    let b =
        workspace::create_card_persisted(cwd.clone(), Some("claude".to_string()), None).unwrap();
    let c = workspace::create_card_persisted(cwd, Some("claude".to_string()), None).unwrap();
    assert_eq!(
        (a.number, b.number, c.number),
        (1, 2, 3),
        "frisk proces: 1,2,3"
    );

    // Luk 2 og 3 — hullet ligger i ENDEN af nummerrummet (den flade der
    // ville genbruge numre, hvis taelleren ikke blev genindlaest).
    workspace::close_card_persisted(b.name).expect("close card-2");
    workspace::close_card_persisted(c.name).expect("close card-3");
}

#[test]
fn worker_restart_b() {
    if !is_worker("worker_restart_b") {
        return;
    }
    let home = home_from_env();
    workspace::startup_load().expect("startup_load after restart");

    // Tom canvas hver gang: session A's kort og taeller er VAEK.
    let ws = workspace::get_workspace().expect("get_workspace");
    assert!(
        ws.cards.is_empty(),
        "session A's card-1 maa ikke genskabes: {ws:?}"
    );
    assert_eq!(
        ws.next_card_number, 1,
        "taelleren starter forfra ved launch"
    );

    let cwd = home.join("work").display().to_string();
    let info = workspace::create_card_persisted(cwd, Some("claude".to_string()), None)
        .expect("create after restart");
    assert_eq!(
        info.number, 1,
        "frisk session -> nummer 1 igen — fik {}",
        info.number
    );
    assert_eq!(info.name, "card-1");
}

// ---------- (f) batch-close + in-session sequence-reset ----------

#[test]
fn batch_close_all_resets_sequence_and_workspace() {
    let home_dir = tempfile::tempdir().unwrap();
    run_worker(
        "worker_batch_close_all_resets_sequence_and_workspace",
        home_dir.path(),
    );
}

#[test]
fn worker_batch_close_all_resets_sequence_and_workspace() {
    if !is_worker("worker_batch_close_all_resets_sequence_and_workspace") {
        return;
    }
    let home = home_from_env();
    workspace::startup_load().expect("startup_load");
    let cwd = home.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd_string = cwd.display().to_string();

    // Supervisionens seedede master lever uden for workspace og maa hverken
    // optage card-1 eller blokere generated-reset.
    let master = CardConfig {
        name: "master".to_string(),
        cwd: cwd.clone(),
        command: vec!["cmd".to_string(), "/c".to_string(), "echo".to_string()],
        resume_command: vec!["cmd".to_string(), "/c".to_string(), "echo".to_string()],
    };
    let master_info = registry::seed_card(master, "claude").expect("seed master");
    assert_eq!(
        master_info.number, 0,
        "master bruger den reserverede non-generated plads"
    );

    let first: Vec<_> = (0..4)
        .map(|_| {
            workspace::create_card_persisted(cwd_string.clone(), Some("claude".to_string()), None)
                .expect("create initial batch")
        })
        .collect();
    assert_eq!(
        first.iter().map(|card| card.number).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );

    let result =
        workspace::close_cards_persisted(first.iter().map(|card| card.name.clone()).collect())
            .expect("close full batch");
    assert_eq!(result.closed, vec!["card-1", "card-2", "card-3", "card-4"]);
    assert!(
        result.errors.is_empty(),
        "batch errors: {:?}",
        result.errors
    );
    assert!(result.sequence_reset, "master maa ikke blokere reset");

    let empty = workspace::get_workspace().expect("workspace after full close");
    assert!(empty.cards.is_empty());
    assert_eq!(empty.next_card_number, 1);
    let on_disk = read_json(&home.join("workspace.json"));
    assert_eq!(on_disk["cards"].as_array().unwrap().len(), 0);
    assert_eq!(on_disk["next_card_number"], 1);
    assert!(
        registry::list_cards()
            .iter()
            .any(|card| card.name == "master"),
        "master bliver staaende uden at blokere generated-sekvensen"
    );

    let second: Vec<_> = (0..3)
        .map(|_| {
            workspace::create_card_persisted(cwd_string.clone(), Some("claude".to_string()), None)
                .expect("create second batch")
        })
        .collect();
    assert_eq!(
        second.iter().map(|card| card.number).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "ny batch starter igen ved 1"
    );
}

#[test]
fn partial_batch_close_does_not_reset_sequence() {
    let home_dir = tempfile::tempdir().unwrap();
    run_worker(
        "worker_partial_batch_close_does_not_reset_sequence",
        home_dir.path(),
    );
}

#[test]
fn worker_partial_batch_close_does_not_reset_sequence() {
    if !is_worker("worker_partial_batch_close_does_not_reset_sequence") {
        return;
    }
    let home = home_from_env();
    workspace::startup_load().expect("startup_load");
    let cwd = home.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.display().to_string();

    let cards: Vec<_> = (0..4)
        .map(|_| {
            workspace::create_card_persisted(cwd.clone(), Some("claude".to_string()), None)
                .expect("create initial batch")
        })
        .collect();
    let result =
        workspace::close_cards_persisted(vec![cards[1].name.clone(), cards[3].name.clone()])
            .expect("partial close");
    assert_eq!(result.closed, vec!["card-2", "card-4"]);
    assert!(result.errors.is_empty());
    assert!(!result.sequence_reset, "card-1/card-3 lever stadig");

    let workspace_after = workspace::get_workspace().expect("workspace after partial close");
    assert_eq!(workspace_after.next_card_number, 5);
    assert_eq!(
        workspace_after
            .cards
            .iter()
            .map(|card| card.name.as_str())
            .collect::<Vec<_>>(),
        vec!["card-1", "card-3"]
    );
    // Laveste-ledige-allokatoren (ejer-beslutning 2026-07-20): det naeste
    // kort genbruger det laveste lukkede nummer — ogsaa gennem den
    // workspace-persisterede sti.
    let next = workspace::create_card_persisted(cwd, Some("claude".to_string()), None)
        .expect("create after partial close");
    assert_eq!((next.number, next.name.as_str()), (2, "card-2"));
}

#[test]
fn concurrent_distinct_closes_preserve_single_sequence_reset() {
    let home_dir = tempfile::tempdir().unwrap();
    run_worker(
        "worker_concurrent_distinct_closes_preserve_single_sequence_reset",
        home_dir.path(),
    );
}

#[test]
fn worker_concurrent_distinct_closes_preserve_single_sequence_reset() {
    if !is_worker("worker_concurrent_distinct_closes_preserve_single_sequence_reset") {
        return;
    }
    let home = home_from_env();
    workspace::startup_load().expect("startup_load");
    let cwd = home.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.display().to_string();
    let cards: Vec<_> = (0..4)
        .map(|_| {
            workspace::create_card_persisted(cwd.clone(), Some("claude".to_string()), None)
                .expect("create concurrent-close fixture")
        })
        .collect();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(cards.len() + 1));
    let workers: Vec<_> = cards
        .into_iter()
        .map(|card| {
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                workspace::close_cards_persisted(vec![card.name])
            })
        })
        .collect();
    barrier.wait();

    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| {
            worker
                .join()
                .expect("close thread panicked")
                .expect("close failed")
        })
        .collect();
    assert!(
        results.iter().all(|result| result.errors.is_empty()),
        "concurrent close errors: {:?}",
        results
            .iter()
            .flat_map(|result| result.errors.iter())
            .collect::<Vec<_>>()
    );
    assert!(
        results.iter().all(|result| result.closed.len() == 1),
        "each distinct close must detach exactly its own card"
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| result.sequence_reset)
            .count(),
        1,
        "exactly the transaction that commits the empty workspace reports reset"
    );

    let workspace_after = workspace::get_workspace().expect("workspace after concurrent closes");
    assert!(workspace_after.cards.is_empty());
    assert_eq!(workspace_after.next_card_number, 1);
    assert!(registry::list_cards().is_empty());
    let on_disk = read_json(&home.join("workspace.json"));
    assert_eq!(on_disk["cards"].as_array().unwrap().len(), 0);
    assert_eq!(on_disk["next_card_number"], 1);
}

// ===========================================================================
// B-light T4 — settings globaliseres (settings.json + WorkspaceResponse-DTO)
// ===========================================================================

fn global_from_env() -> PathBuf {
    PathBuf::from(std::env::var("TALMINAL_GLOBAL_HOME").expect("worker needs TALMINAL_GLOBAL_HOME"))
}

/// Efter set_settings: workspace.json UDEN settings-felt; settings.json MED.
#[test]
fn settings_bor_globalt_ikke_i_workspace_json() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path();
    let global = global_dir.path();

    run_worker_with_global("worker_settings_global", home, Some(global));

    let ws = read_json(&home.join("workspace.json"));
    assert!(
        ws.get("settings").is_none(),
        "persist-formen maa ikke baere settings: {ws}"
    );
    let settings = read_json(&global.join("settings.json"));
    assert_eq!(settings["ptt_hotkey"], "Alt+F11");
    assert_eq!(settings["exit_type_mode_hotkey"], "Ctrl+F23");
    assert_eq!(settings["wallpaper"], "blue-folds");
}

#[test]
fn worker_settings_global() {
    if !is_worker("worker_settings_global") {
        return;
    }
    let _home = home_from_env();
    let _global = global_from_env();
    workspace::startup_load().expect("startup_load");
    workspace::set_settings(settings_input(
        "Alt+F11",
        "Ctrl+F23",
        "pipeline",
        "blue-folds",
        "claude",
    ))
    .expect("set_settings");
    // Tom start skriver ingen fil ved launch — fremtving en workspace-persist,
    // saa foraeldre-testen kan verificere at settings IKKE bor i persist-formen.
    workspace::set_viewport(1.0, 2.0, 1.0).expect("set_viewport");
    workspace::persist_now().expect("persist_now");
}

/// Tomme/whitespace-bindings afvises — og afvisning persisterer intet.
#[test]
fn set_settings_afviser_tomme_og_whitespace_bindings() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global(
        "worker_settings_validation",
        home_dir.path(),
        Some(global_dir.path()),
    );
    assert!(
        !global_dir.path().join("settings.json").exists(),
        "afviste bindings maa ikke skrive settings.json"
    );
}

#[test]
fn worker_settings_validation() {
    if !is_worker("worker_settings_validation") {
        return;
    }
    let _home = home_from_env();
    let _global = global_from_env();
    workspace::startup_load().expect("startup_load");
    assert!(
        workspace::set_settings(settings_input(
            "",
            "Ctrl+F7",
            "realtime",
            "blue-folds",
            "claude",
        ))
        .is_err(),
        "tom ptt_hotkey skal afvises"
    );
    assert!(
        workspace::set_settings(settings_input(
            "x",
            "   ",
            "realtime",
            "blue-folds",
            "claude",
        ))
        .is_err(),
        "whitespace-only exit-hotkey skal afvises"
    );
}

/// Wire-formen (get_workspace) komponerer persist-fil + load_settings().
#[test]
fn get_workspace_response_baerer_settings() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global(
        "worker_get_workspace_response",
        home_dir.path(),
        Some(global_dir.path()),
    );
}

#[test]
fn workspace_response_carries_the_resolved_routes_but_settings_does_not() {
    // `stt_provider` er med vilje sat til et slug der IKKE findes mere:
    // OpenRouter-STT blev slettet 2026-07-29, og en eksisterende settings.json
    // ude i verden kan sagtens stadig have værdien stående. Den tolerante
    // læse-side i `resolve_voice_routes` skal degradere den til default-ruten
    // — ikke fejle, og ikke efterlade brugeren uden stemme. Det er hele
    // migreringen: der er ingen.
    let settings = talminal_canvas_lib::workspace::Settings {
        stt_provider: "openrouter".to_string(),
        routing_provider: "google".to_string(),
        ..Default::default()
    };
    let json = serde_json::to_string(&settings).expect("serialize settings");
    assert!(!json.contains("endpoint"));

    let routes = talminal_canvas_lib::workspace::resolve_voice_routes(&settings);
    assert_eq!(
        routes.stt.slug, "openai",
        "ukendt stt-slug skal falde tilbage"
    );
    assert_eq!(routes.stt.model, "gpt-transcribe");
    assert!(routes.stt.supports_partials);
    // Routing-OpenRouter lever videre — det var kun STT-ruten der forsvandt.
    assert_eq!(routes.routing.slug, "google");
    assert_eq!(routes.routing.model, "gemini-3.1-flash-lite");
}

/// Afloeser `mini_slug_resolves_all_the_way_to_the_mini_model`, som forsvandt
/// med mini-ruten 2026-08-05. Hvor den gamle test beviste at slug'et naaede
/// FREM til mini-modellen, beviser denne at det ikke laengere kan.
///
/// Det er den eneste automatiske daekning af migreringen. En bruger der har
/// valgt mini har `"openai-mini"` staaende i sin settings.json lige nu, og der
/// skrives ikke tilbage ved indlaesning — vaerdien bliver staaende for evigt,
/// indtil noget andet gemmes. Falder denne test, taler den bruger til en model
/// der ikke findes, og fejlen viser sig som en stum mikrofon.
#[test]
fn the_retired_mini_slug_falls_back_to_the_only_route() {
    let settings = talminal_canvas_lib::workspace::Settings {
        stt_provider: "openai-mini".to_string(),
        ..Default::default()
    };

    let routes = talminal_canvas_lib::workspace::resolve_voice_routes(&settings);
    assert_eq!(routes.stt.slug, "openai", "nedlagt slug skal falde tilbage");
    assert_eq!(routes.stt.model, "gpt-transcribe");
    assert_eq!(routes.stt.key_slot, "provider_key_openai");
    assert_eq!(
        routes.stt.endpoint,
        "wss://api.openai.com/v1/realtime?intent=transcription"
    );
    assert!(routes.stt.supports_partials);
    assert!(routes.stt.supports_domain_prompt);
}

#[test]
fn worker_get_workspace_response() {
    if !is_worker("worker_get_workspace_response") {
        return;
    }
    let _home = home_from_env();
    let _global = global_from_env();
    workspace::startup_load().expect("startup_load");

    let before = workspace::get_workspace().expect("get_workspace defaults");
    assert_eq!(before.settings, Settings::default());
    assert_eq!(before.settings.ptt_hotkey, DEFAULT_PTT_HOTKEY);
    assert_eq!(
        before.settings.exit_type_mode_hotkey,
        DEFAULT_EXIT_TYPE_MODE_HOTKEY
    );

    workspace::set_settings(settings_input(
        "Ctrl+F10",
        "Alt+F21",
        "pipeline",
        "liquid-only",
        "claude",
    ))
    .expect("set_settings");
    let after = workspace::get_workspace().expect("get_workspace after set");
    assert_eq!(after.settings.ptt_hotkey, "Ctrl+F10");
    assert_eq!(after.settings.exit_type_mode_hotkey, "Alt+F21");
    assert_eq!(after.settings.voice_engine, "pipeline");
    assert_eq!(after.settings.wallpaper, "liquid-only");
    // Persist-felter er stadig der (wire-formen er uændret for frontenden).
    assert_eq!(after.schema_version, 1);
    assert_eq!(after.next_card_number, before.next_card_number);
    assert_eq!(after.viewport, before.viewport);
    assert_eq!(after.cards, before.cards);
}

/// Fravær/korrupt settings.json → Settings::default() (mildere end project.json).
#[test]
fn load_settings_default_ved_fravaer_og_korrupt() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global(
        "worker_load_settings_default",
        home_dir.path(),
        Some(global_dir.path()),
    );
}

#[test]
fn worker_load_settings_default() {
    if !is_worker("worker_load_settings_default") {
        return;
    }
    let _home = home_from_env();
    let global = global_from_env();

    assert_eq!(
        settings_path(),
        global.join("settings.json"),
        "settings_path = global_base()/settings.json"
    );
    assert!(
        !settings_path().exists(),
        "fravær: ingen settings.json endnu"
    );
    assert_eq!(load_settings(), Settings::default(), "fravær => defaults");

    std::fs::create_dir_all(&global).unwrap();
    std::fs::write(global.join("settings.json"), b"{ not valid json !!!").unwrap();
    assert_eq!(
        load_settings(),
        Settings::default(),
        "korrupt => defaults (ikke fatal Err)"
    );
}

/// To TALMINAL_HOME-workers deler én TALMINAL_GLOBAL_HOME / settings.json.
#[test]
fn to_projekter_deler_settings() {
    let home_a = tempfile::tempdir().unwrap();
    let home_b = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    let global = global_dir.path();

    run_worker_with_global("worker_settings_writer", home_a.path(), Some(global));
    run_worker_with_global("worker_settings_reader", home_b.path(), Some(global));

    // Begge projekter har egen workspace.json — settings bor kun globalt.
    assert!(home_a.path().join("workspace.json").is_file());
    assert!(home_b.path().join("workspace.json").is_file());
    let settings = read_json(&global.join("settings.json"));
    assert_eq!(settings["ptt_hotkey"], "Shift+F8");
    assert_eq!(settings["exit_type_mode_hotkey"], "Ctrl+F19");
}

#[test]
fn worker_settings_writer() {
    if !is_worker("worker_settings_writer") {
        return;
    }
    let _home = home_from_env();
    let _global = global_from_env();
    workspace::startup_load().expect("startup_load");
    workspace::set_settings(settings_input(
        "Shift+F8",
        "Ctrl+F19",
        "pipeline",
        "blue-folds",
        "claude",
    ))
    .expect("set_settings");
    // Fremtving workspace-persist (tom start skriver ellers ingen fil).
    workspace::set_viewport(1.0, 2.0, 1.0).expect("set_viewport");
    workspace::persist_now().expect("persist_now");
}

#[test]
fn worker_settings_reader() {
    if !is_worker("worker_settings_reader") {
        return;
    }
    let _home = home_from_env();
    let _global = global_from_env();
    workspace::startup_load().expect("startup_load");
    let ws = workspace::get_workspace().expect("get_workspace");
    assert_eq!(ws.settings.ptt_hotkey, "Shift+F8");
    assert_eq!(ws.settings.exit_type_mode_hotkey, "Ctrl+F19");
    assert_eq!(ws.settings.voice_engine, "pipeline");
    // Fremtving workspace-persist (foraeldre-testen asserter fil-eksistens).
    workspace::set_viewport(1.0, 2.0, 1.0).expect("set_viewport");
    workspace::persist_now().expect("persist_now");
}

/// Gamle workspace.json-filer med settings-felt skal stadig loade (ingen
/// deny_unknown_fields — feltet ignoreres i persist-formen).
#[test]
fn gammel_workspace_fil_med_settings_felt_loader() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    std::fs::write(
        &path,
        r#"{
  "schema_version": 1,
  "next_card_number": 2,
  "viewport": { "x": 1.0, "y": 2.0, "zoom": 1.5 },
  "settings": {
    "ptt_hotkey": "Alt+F24",
    "exit_type_mode_hotkey": "Ctrl+F23"
  },
  "cards": []
}
"#,
    )
    .unwrap();

    let file = load_workspace_file(&path)
        .expect("gammel fil m. settings-felt skal loade")
        .expect("file exists");
    assert_eq!(file.schema_version, 1);
    assert_eq!(file.next_card_number, 2);
    assert_eq!(file.viewport.zoom, 1.5);
    assert!(file.cards.is_empty());
}

#[test]
fn settings_default_voice_engine_is_pipeline() {
    assert_eq!(Settings::default().voice_engine, "pipeline");
    let legacy: Settings =
        serde_json::from_str(r#"{"ptt_hotkey":"Ctrl+F12","exit_type_mode_hotkey":"Shift+Escape"}"#)
            .expect("legacy settings deserialize");
    assert_eq!(legacy.voice_engine, "pipeline");
}

#[test]
fn normalize_settings_maps_realtime_engine_to_pipeline() {
    let settings = Settings {
        voice_engine: "realtime".to_string(),
        ..Default::default()
    };
    let normalized = workspace::normalize_settings(settings);
    assert_eq!(normalized.voice_engine, "pipeline");
}

#[test]
fn normalize_settings_maps_unknown_engine_to_pipeline() {
    for raw in ["", "  ", "REALTIME", "eksperiment"] {
        let settings = Settings {
            voice_engine: raw.to_string(),
            ..Default::default()
        };
        assert_eq!(
            workspace::normalize_settings(settings).voice_engine,
            "pipeline",
            "engine {raw:?} skulle normaliseres til pipeline"
        );
    }
}

#[test]
fn normalize_settings_defaults_missing_routes() {
    let settings = Settings::default();
    assert_eq!(settings.stt_provider, "openai");
    assert_eq!(settings.routing_provider, "vercel");
}

#[test]
fn normalize_settings_tolerates_bad_route_slugs() {
    for raw in ["", "   ", "OPENAI", "deepgram"] {
        let settings = Settings {
            stt_provider: raw.to_string(),
            ..Default::default()
        };
        assert_eq!(
            workspace::normalize_settings(settings).stt_provider,
            "openai",
            "stt_provider {raw:?} skulle falde tilbage til default"
        );
    }
    for raw in ["", "   ", "Vercel", "anthropic"] {
        let settings = Settings {
            routing_provider: raw.to_string(),
            ..Default::default()
        };
        assert_eq!(
            workspace::normalize_settings(settings).routing_provider,
            "vercel",
            "routing_provider {raw:?} skulle falde tilbage til default"
        );
    }
}

/// Trimning må ikke forveksles med fallback: en GYLDIG rute med mellemrum
/// omkring skal trimmes og beholdes, ikke smides ud som ukendt.
///
/// Eksemplet var "  openrouter  " på stt_provider indtil 2026-07-29, hvor
/// OpenRouter-STT blev slettet — så blev det et fallback-tilfælde og målte
/// noget andet end testens navn. Routing-siden har stadig flere gyldige ruter,
/// og "  google  " er derfor et ægte trim-tilfælde.
#[test]
fn normalize_settings_trims_padded_but_valid_route() {
    let settings = Settings {
        routing_provider: "  google  ".to_string(),
        ..Default::default()
    };
    assert_eq!(
        workspace::normalize_settings(settings).routing_provider,
        "google"
    );
}

#[test]
fn settings_input_rejects_a_missing_field() {
    // Hvert af de to json'er mangler PRAECIS eet felt, saa testen ikke kan
    // bestaa af den forkerte grund naar wire-formen vokser.
    let uden_routing = r#"{"ptt_hotkey":"CmdOrCtrl+Shift+Space","exit_type_mode_hotkey":"Shift+Escape","voice_engine":"pipeline","wallpaper":"blue-folds","default_agent":"claude","stt_provider":"openai","dictation_hotkey":"CmdOrCtrl+Shift+KeyD","dictation_submit":false}"#;
    assert!(
        serde_json::from_str::<SettingsInput>(uden_routing).is_err(),
        "manglende routing_provider skulle vaere en deserialiseringsfejl"
    );

    let uden_dictation = r#"{"ptt_hotkey":"CmdOrCtrl+Shift+Space","exit_type_mode_hotkey":"Shift+Escape","voice_engine":"pipeline","wallpaper":"blue-folds","default_agent":"claude","stt_provider":"openai","routing_provider":"vercel","dictation_submit":false}"#;
    assert!(
        serde_json::from_str::<SettingsInput>(uden_dictation).is_err(),
        "manglende dictation_hotkey skulle vaere en deserialiseringsfejl"
    );

    let uden_keywords = r#"{"ptt_hotkey":"CmdOrCtrl+Shift+Space","exit_type_mode_hotkey":"Shift+Escape","voice_engine":"pipeline","wallpaper":"blue-folds","default_agent":"claude","stt_provider":"openai","routing_provider":"vercel","dictation_hotkey":"CmdOrCtrl+Shift+KeyD","dictation_submit":false}"#;
    assert!(
        serde_json::from_str::<SettingsInput>(uden_keywords).is_err(),
        "manglende stt_keywords skulle vaere en deserialiseringsfejl"
    );

    // Kontrolproeve: med ALLE felter parser den.
    let komplet = r#"{"ptt_hotkey":"CmdOrCtrl+Shift+Space","exit_type_mode_hotkey":"Shift+Escape","voice_engine":"pipeline","wallpaper":"blue-folds","default_agent":"claude","stt_provider":"openai","routing_provider":"vercel","dictation_hotkey":"CmdOrCtrl+Shift+KeyD","dictation_submit":false,"stt_keywords":["TalminalMCP"]}"#;
    serde_json::from_str::<SettingsInput>(komplet).expect("komplet wire-form skal parse");
}

/// En settings.json fra foer feltet fandtes skal give STANDARDLISTEN, ikke en
/// tom liste.
///
/// Det haenger paa at feltet IKKE har sit eget `#[serde(default)]`: structen
/// har attributten paa container-niveau, og en felt-attribut ville vinde over
/// den og resolve til `Vec::default()` = tom. Skriver nogen kortformen paa
/// feltet i god tro, mister hver eksisterende bruger sin liste uden en fejl —
/// og denne test er det eneste sted det falder.
#[test]
fn gammel_settings_json_uden_keywords_faar_standardlisten() {
    let legacy: Settings =
        serde_json::from_str(r#"{"ptt_hotkey":"CmdOrCtrl+Shift+Space","wallpaper":"blue-folds"}"#)
            .expect("legacy settings skal parse");
    assert_eq!(
        legacy.stt_keywords,
        Settings::default().stt_keywords,
        "manglende felt skal give standardlisten, ikke tom"
    );
    assert!(legacy.stt_keywords.contains(&"TalminalMCP".to_string()));
}

/// En BEVIDST ryddet liste skal overleve normaliseringen.
///
/// Dette bryder med `normalized_slug`-konventionen lige ved siden af, hvor tom
/// altid falder tilbage til defaulten. Bruddet er med vilje: en tom
/// keyword-liste er et valg, og et valg der bliver overskrevet ved hver
/// indlaesning er ikke et valg. Spejler nogen nabokoden og skriver "tom =>
/// default", faelder denne test det.
#[test]
fn en_bevidst_tom_keyword_liste_overlever() {
    let ryddet: Settings =
        serde_json::from_str(r#"{"stt_keywords":[]}"#).expect("tom liste skal parse");
    assert!(
        ryddet.stt_keywords.is_empty(),
        "tom liste skal parse som tom"
    );

    let efter = talminal_canvas_lib::workspace::normalize_settings(ryddet);
    assert!(
        efter.stt_keywords.is_empty(),
        "normaliseringen maa ikke give standardlisten tilbage"
    );
}

/// Rensningen: trim, forbudte tegn ud, tomme droppet, loft haandhaevet.
///
/// `<` `>` CR og LF er dem OpenAIs docs forbyder. De STRIPPES frem for at
/// afvises — at faa en gemning afvist fordi man kom til at skrive et "<" er
/// ikke en bedre oplevelse end at tegnet forsvinder.
#[test]
fn keywords_renses_men_afvises_ikke() {
    let raw: Vec<String> = vec![
        "  TalminalMCP  ".to_string(),
        "Cod<ex>".to_string(),
        String::new(),
        "   ".to_string(),
        "linje\nskift".to_string(),
    ];
    let renset = talminal_canvas_lib::workspace::normalize_keywords(&raw);
    assert_eq!(
        renset,
        vec![
            "TalminalMCP".to_string(),
            "Codex".to_string(),
            "linjeskift".to_string(),
        ]
    );

    // Loftet er VORES, ikke API'ets: docs siger kun "maximum length enforced
    // by model". Testen laaser at der ER et loft, ikke at 100 er rigtigt.
    let mange: Vec<String> = (0..250).map(|i| format!("ord{i}")).collect();
    assert_eq!(
        talminal_canvas_lib::workspace::normalize_keywords(&mange).len(),
        100
    );
}

#[test]
fn gammel_settings_json_uden_dikterings_felter_faar_defaults() {
    let legacy: Settings =
        serde_json::from_str(r#"{"ptt_hotkey":"Ctrl+F12","exit_type_mode_hotkey":"Shift+Escape"}"#)
            .expect("legacy settings deserialize");
    assert_eq!(legacy.dictation_hotkey, workspace::DEFAULT_DICTATION_HOTKEY);
    assert!(
        !legacy.dictation_submit,
        "auto-send skal vaere FRA for den der opgraderer"
    );
}

#[test]
fn set_settings_afviser_kolliderende_genveje() {
    let home = tempfile::tempdir().expect("home");
    let global = tempfile::tempdir().expect("global");
    run_worker_with_global(
        "worker_set_settings_afviser_kolliderende_genveje",
        home.path(),
        Some(global.path()),
    );
    // Workeren afviser to kolliderende kombinationer og gemmer derefter EEN
    // gyldig. At filen findes med den gyldige genvej er beviset for at
    // afvisningerne var kirurgiske — ikke at hele set_settings var doed.
    let saved = std::fs::read_to_string(global.path().join("settings.json"))
        .expect("kontrolproeven skal have gemt en settings.json");
    assert!(
        saved.contains("CmdOrCtrl+Shift+KeyD"),
        "uventet settings.json: {saved}"
    );
}

#[test]
fn worker_set_settings_afviser_kolliderende_genveje() {
    if !is_worker("worker_set_settings_afviser_kolliderende_genveje") {
        return;
    }
    // Ingen `common::serial()` her: worker-testen koerer i sin EGEN proces med
    // TALMINAL_HOME/_GLOBAL_HOME allerede sat af `run_worker_with_global`, og
    // serial() ville pege dem et andet sted hen end det driveren asserter paa.

    // Identiske genveje.
    let mut ens = settings_input(
        "CmdOrCtrl+Shift+Space",
        "Shift+Escape",
        "pipeline",
        "blue-folds",
        "claude",
    );
    ens.dictation_hotkey = "CmdOrCtrl+Shift+Space".to_string();
    let error = workspace::set_settings(ens).expect_err("ens genveje skal afvises");
    assert!(error.contains("samme tast"), "uventet fejltekst: {error}");

    // SUBSET-faelden, og den vigtigste af de to: de to strenge er FORSKELLIGE,
    // men eet Ctrl+Shift+Space-tryk matcher dem begge, fordi combo-matchet er
    // subset og ikke lighed.
    let mut subset = settings_input(
        "CmdOrCtrl+Space",
        "Shift+Escape",
        "pipeline",
        "blue-folds",
        "claude",
    );
    subset.dictation_hotkey = "CmdOrCtrl+Shift+Space".to_string();
    let error = workspace::set_settings(subset).expect_err("subset-kollision skal afvises");
    assert!(error.contains("samme tast"), "uventet fejltekst: {error}");

    // Kontrolproeve: forskellige trigger-taster gemmes fint.
    let mut ok = settings_input(
        "CmdOrCtrl+Shift+Space",
        "Shift+Escape",
        "pipeline",
        "blue-folds",
        "claude",
    );
    ok.dictation_hotkey = "CmdOrCtrl+Shift+KeyD".to_string();
    ok.dictation_submit = true;
    workspace::set_settings(ok).expect("forskellige taster skal gemmes");
    // `load_settings()` frem for `get_workspace()`: settings.json er global og
    // uafhaengig af om et workspace er loadet — og det er kun settings vi maaler.
    let saved = load_settings();
    assert_eq!(saved.dictation_hotkey, "CmdOrCtrl+Shift+KeyD");
    assert!(saved.dictation_submit);
}

#[test]
fn set_settings_rejects_unknown_stt_provider() {
    let home = tempfile::tempdir().expect("home");
    let global = tempfile::tempdir().expect("global");
    run_worker_with_global(
        "worker_set_settings_rejects_unknown_stt_provider",
        home.path(),
        Some(global.path()),
    );
    assert!(!global.path().join("settings.json").exists());
}

#[test]
fn worker_set_settings_rejects_unknown_stt_provider() {
    if !is_worker("worker_set_settings_rejects_unknown_stt_provider") {
        return;
    }
    let mut bad = settings_input(
        "CmdOrCtrl+Shift+Space",
        "Shift+Escape",
        "pipeline",
        "blue-folds",
        "claude",
    );
    bad.stt_provider = "deepgram".to_string();
    let error = workspace::set_settings(bad).expect_err("ukendt stt_provider skal afvises");
    assert!(error.contains("stt_provider"));
}

#[test]
fn settings_json_without_wallpaper_uses_default() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global(
        "worker_settings_json_without_wallpaper_uses_default",
        home_dir.path(),
        Some(global_dir.path()),
    );
}

#[test]
fn worker_settings_json_without_wallpaper_uses_default() {
    if !is_worker("worker_settings_json_without_wallpaper_uses_default") {
        return;
    }
    let _home = home_from_env();
    let global = global_from_env();
    std::fs::write(
        global.join("settings.json"),
        r#"{
  "ptt_hotkey": "Ctrl+F12",
  "exit_type_mode_hotkey": "Shift+Escape",
  "voice_engine": "pipeline"
}"#,
    )
    .unwrap();

    let settings = load_settings();
    assert_eq!(settings.wallpaper, "blue-folds");
    assert_eq!(settings.ptt_hotkey, "Ctrl+F12");
}

#[test]
fn set_settings_rejects_unknown_engine() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global(
        "worker_set_settings_rejects_unknown_engine",
        home_dir.path(),
        Some(global_dir.path()),
    );
    assert!(!global_dir.path().join("settings.json").exists());
}

#[test]
fn worker_set_settings_rejects_unknown_engine() {
    if !is_worker("worker_set_settings_rejects_unknown_engine") {
        return;
    }
    let _home = home_from_env();
    let _global = global_from_env();
    for engine in ["unknown", "realtime"] {
        let result = workspace::set_settings(settings_input(
            "Ctrl+F12",
            "Shift+Escape",
            engine,
            "blue-folds",
            "claude",
        ));
        assert!(result.is_err(), "{engine:?} skal afvises");
    }
}

#[test]
fn set_settings_rejects_unknown_wallpaper() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global(
        "worker_set_settings_rejects_unknown_wallpaper",
        home_dir.path(),
        Some(global_dir.path()),
    );
    assert!(!global_dir.path().join("settings.json").exists());
}

#[test]
fn worker_set_settings_rejects_unknown_wallpaper() {
    if !is_worker("worker_set_settings_rejects_unknown_wallpaper") {
        return;
    }
    let _home = home_from_env();
    let _global = global_from_env();
    let result = workspace::set_settings(settings_input(
        "Ctrl+F12",
        "Shift+Escape",
        "pipeline",
        "not-bundled",
        "claude",
    ));
    assert!(result.is_err());
}

/// Skrive-siden er STRENG (i modsaetning til `load_settings`s tolerante
/// normalisering, T5 review-fund 2): en ukendt `default_agent`-slug afvises
/// med en `Err`, og afvisningen persisterer intet.
#[test]
fn set_settings_rejects_unknown_default_agent() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global(
        "worker_set_settings_rejects_unknown_default_agent",
        home_dir.path(),
        Some(global_dir.path()),
    );
    assert!(!global_dir.path().join("settings.json").exists());
}

#[test]
fn worker_set_settings_rejects_unknown_default_agent() {
    if !is_worker("worker_set_settings_rejects_unknown_default_agent") {
        return;
    }
    let _home = home_from_env();
    let _global = global_from_env();
    let result = workspace::set_settings(settings_input(
        "Ctrl+F12",
        "Shift+Escape",
        "pipeline",
        "blue-folds",
        "cursor",
    ));
    assert!(result.is_err());
}

/// K2-choke-point (T5 review-fund 1): `create_card_persisted(profile: None)`
/// skal laese den LIVE `default_agent`-setting, ikke en hardcoded konstant.
/// Isoleret worker-proces (egen TALMINAL_GLOBAL_HOME) — ellers ville testen
/// stille afhaenge af udviklerens rigtige %LOCALAPPDATA%\Talminal\settings.json
/// (`project::global_base()`s fallback), som Task 7's picker snart kan aendre.
#[test]
fn create_card_without_profile_uses_default_agent_setting() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global(
        "worker_create_card_without_profile_uses_default_agent_setting",
        home_dir.path(),
        Some(global_dir.path()),
    );
}

#[test]
fn worker_create_card_without_profile_uses_default_agent_setting() {
    if !is_worker("worker_create_card_without_profile_uses_default_agent_setting") {
        return;
    }
    let home = home_from_env();
    let _global = global_from_env();
    workspace::startup_load().expect("startup_load");
    // Saet default_agent til "codex" (uden default-vaerdien) — beviser at
    // choke-pointet reelt LAESER settingen frem for at falde tilbage til en
    // hardcoded "claude"-konstant.
    workspace::set_settings(settings_input(
        DEFAULT_PTT_HOTKEY,
        DEFAULT_EXIT_TYPE_MODE_HOTKEY,
        "pipeline",
        "blue-folds",
        "codex",
    ))
    .expect("set_settings default_agent=codex");

    let cwd = home.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let info = workspace::create_card_persisted(cwd.display().to_string(), None, None)
        .expect("create without explicit profile");
    assert_eq!(
        info.profile, "codex",
        "profile: None skal resolve til default_agent-settingen (\"codex\"), ikke en hardcoded default"
    );
}

#[test]
fn settings_roundtrip_persists_engine() {
    let home_dir = tempfile::tempdir().unwrap();
    let global_dir = tempfile::tempdir().unwrap();
    run_worker_with_global(
        "worker_settings_roundtrip_engine",
        home_dir.path(),
        Some(global_dir.path()),
    );
    let settings = read_json(&global_dir.path().join("settings.json"));
    assert_eq!(settings["voice_engine"], "pipeline");
    assert_eq!(settings["wallpaper"], "blue-folds");
}

#[test]
fn worker_settings_roundtrip_engine() {
    if !is_worker("worker_settings_roundtrip_engine") {
        return;
    }
    let _home = home_from_env();
    let _global = global_from_env();
    workspace::set_settings(settings_input(
        "Ctrl+F12",
        "Shift+Escape",
        "pipeline",
        "blue-folds",
        "claude",
    ))
    .expect("set_settings");
    assert_eq!(load_settings().voice_engine, "pipeline");
    assert_eq!(load_settings().wallpaper, "blue-folds");
}

#[test]
fn set_settings_afviser_uparsebar_ptt_hotkey() {
    let home = tempfile::tempdir().expect("home");
    let global = tempfile::tempdir().expect("global");
    run_worker_with_global(
        "worker_set_settings_afviser_uparsebar_ptt_hotkey",
        home.path(),
        Some(global.path()),
    );
}

#[test]
fn worker_set_settings_afviser_uparsebar_ptt_hotkey() {
    if !is_worker("worker_set_settings_afviser_uparsebar_ptt_hotkey") {
        return;
    }
    let err = workspace::set_settings(settings_input(
        "Gib",
        "Shift+Escape",
        "pipeline",
        "blue-folds",
        "claude",
    ))
    .expect_err("uparsebar ptt_hotkey skal afvises");
    assert!(err.contains("Gib"), "fejlen skal naevne vaerdien: {err}");
}

#[test]
fn get_workspace_advarer_om_uparsebar_ptt_hotkey_paa_disken() {
    let home = tempfile::tempdir().expect("home");
    let global = tempfile::tempdir().expect("global");
    std::fs::write(
        global.path().join("settings.json"),
        r#"{"ptt_hotkey":"Gib","exit_type_mode_hotkey":"Shift+Escape"}"#,
    )
    .expect("skriv settings.json");
    run_worker_with_global(
        "worker_get_workspace_advarer_om_uparsebar_ptt_hotkey",
        home.path(),
        Some(global.path()),
    );
}

#[test]
fn worker_get_workspace_advarer_om_uparsebar_ptt_hotkey() {
    if !is_worker("worker_get_workspace_advarer_om_uparsebar_ptt_hotkey") {
        return;
    }
    workspace::startup_load().expect("startup_load");
    let ws = workspace::get_workspace().expect("get_workspace");
    assert_eq!(ws.settings.ptt_hotkey, DEFAULT_PTT_HOTKEY);
    let warning = ws.settings_warning.expect("advarsel");
    assert!(
        warning.contains("Gib"),
        "advarslen skal naevne vaerdien: {warning}"
    );
}
