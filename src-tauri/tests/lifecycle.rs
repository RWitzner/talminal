// Task 5 — runtime kort-livscyklus: registry create/close/list (lib-cratet).
//
// Synkrone #[test] (ingen tokio, som de eksisterende integrationstests).
// Registryet er en global singleton, og cargo koerer testene parallelt i
// SAMME proces.
//
// FLAKE 2026-07-26: "parallel-robuste asserts" var ikke nok. Kortnumre
// allokeres som LAVESTE LEDIGE (registry.rs::allocate_generated, ejer-
// beslutning 2026-07-20), saa navnet `card-N` GENOPSTAAR saa snart et andet
// kort faar nummer N. En test der lukker sit kort og derefter asserter at
// navnet er VAEK, asserter altsaa paa en global ressource den ikke ejer:
// en samtidig test kan have genbrugt nummeret i mellemtiden. Bevist ved
// instrumentering — offenderen bar en FREMMED cwd. Under den gamle monotone
// taeller var racet umuligt, saa defekten laa latent.
//
// Kontrakten er derfor nu husets egen: hver test der MUTERER registryet
// (create/seed/close) tager `common::serial()` som foerste linje. Den samme
// laas sandkasser ogsaa TALMINAL_HOME og TALMINAL_GLOBAL_HOME, saa filen
// opfylder plan-invarianten "ingen test skriver i den rigtige datamappe".
// Tests der hverken muterer registryet eller roerer datamappen
// (`readiness_gate_*`, driveren `numbers_reuse_lowest_free_after_close` der
// kun spawner en worker-proces) tager den IKKE — serialisering koster
// koeretid, og de har intet at beskytte.
//
// Absolutte nummer-asserts bruger workspace.rs' worker-proces-moenster:
// driveren spawner test-binaren selv (current_exe) med `--exact <worker>` og
// TALMINAL_WORKER som markoer; uden markoeren er worker-testen en no-op.
// Exit observeres via try_exit_status — ALDRIG via reader-EOF (FUND 11).

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use talminal_canvas_lib::cards::CardConfig;
use talminal_canvas_lib::pty::{PtyHost, PtySpawn};
use talminal_canvas_lib::registry;

// ---------- hjaelpere ----------

fn cmd_exe() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!(r"{root}\System32\cmd.exe")
}

fn create_default(dir: &Path) -> registry::CardInfo {
    registry::create_card(dir.display().to_string(), "claude".to_string(), None)
        .expect("create_card with default profile command")
}

/// Worker-proces-moensteret fra tests/workspace.rs: frisk proces = frisk
/// registry-singleton, saa absolutte kortnumre kan assertes deterministisk.
fn run_worker(worker: &str) {
    let exe = std::env::current_exe().expect("current_exe");
    let out = std::process::Command::new(exe)
        .arg("--exact")
        .arg(worker)
        .arg("--test-threads=1")
        .arg("--nocapture")
        .env("TALMINAL_WORKER", worker)
        .output()
        .expect("spawn worker test process");
    assert!(
        out.status.success(),
        "worker '{worker}' failed\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn is_worker(name: &str) -> bool {
    std::env::var("TALMINAL_WORKER").as_deref() == Ok(name)
}

/// Profilens kommandoer som Vec<String> — testene sammenligner mod PROFILEN
/// (Task 2-kontrakten), ikke mod literals.
fn claude_commands() -> (Vec<String>, Vec<String>) {
    let p = talminal_canvas_lib::profiles::profile("claude").expect("claude profile");
    (
        p.spawn_command.iter().map(|s| s.to_string()).collect(),
        p.resume_command.iter().map(|s| s.to_string()).collect(),
    )
}

// ---------- (a) create -> list roundtrip m. felter ----------

#[test]
fn create_then_list_roundtrip_with_fields() {
    let _serial = common::serial();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().display().to_string();
    let info = registry::create_card(cwd.clone(), "claude".to_string(), None)
        .expect("create_card must succeed for an existing cwd");

    assert!(info.number >= 1, "numre starter ved 1, fik {}", info.number);
    assert_eq!(
        info.name,
        format!("card-{}", info.number),
        "name er den stabile noegle: card-{{number}}"
    );
    assert_eq!(info.cwd, cwd);
    assert_eq!(info.profile, "claude");
    assert!(!info.running, "et nyoprettet kort er ikke spawnet endnu");
    assert_eq!(info.exited, None);

    let listed = registry::list_cards();
    let found = listed
        .iter()
        .find(|c| c.name == info.name)
        .expect("created card must appear in list_cards");
    assert_eq!(
        found, &info,
        "list_cards skal vise samme felter som create returnerede"
    );

    registry::close_card(info.name.clone()).expect("close_card cleanup");
    assert!(
        !registry::list_cards().iter().any(|c| c.name == info.name),
        "closed card must be gone from list_cards"
    );
}

// ---------- (b) laveste ledige nummer genbruges efter close ----------
// (ejer-beslutning 2026-07-20: afloeser den monotone taeller)

#[test]
fn live_cards_have_unique_numbers() {
    let _serial = common::serial();
    let dir = tempfile::tempdir().unwrap();
    let a = create_default(dir.path());
    let b = create_default(dir.path());
    assert_ne!(a.number, b.number, "levende kort deler aldrig nummer");
    registry::close_card(a.name).expect("close a");
    registry::close_card(b.name).expect("close b");
}

#[test]
fn numbers_reuse_lowest_free_after_close() {
    run_worker("worker_numbers_reuse_lowest_free");
}

#[test]
fn worker_numbers_reuse_lowest_free() {
    if !is_worker("worker_numbers_reuse_lowest_free") {
        return;
    }
    // Worker-processen koerer med --test-threads=1 og er alene om registryet,
    // men laasen sandkasser ogsaa datamappen — og env'en er arvet fra driveren.
    let _serial = common::serial();
    let dir = tempfile::tempdir().unwrap();
    let a = create_default(dir.path());
    let b = create_default(dir.path());
    let c = create_default(dir.path());
    let d = create_default(dir.path());
    assert_eq!(
        (a.number, b.number, c.number, d.number),
        (1, 2, 3, 4),
        "frisk proces: numrene starter 1..4"
    );

    // Huller i MIDTEN af nummerrummet: luk 2 og 3.
    registry::close_card(b.name).expect("close b");
    registry::close_card(c.name).expect("close c");

    let e = create_default(dir.path());
    assert_eq!(
        (e.number, e.name.as_str()),
        (2, "card-2"),
        "laveste hul genbruges foerst"
    );
    let f = create_default(dir.path());
    assert_eq!(f.number, 3, "naeste hul derefter");
    let g = create_default(dir.path());
    assert_eq!(
        g.number, 5,
        "ingen huller tilbage -> foerste nummer over de levende"
    );

    // Hul i STARTEN: luk card-1, naeste kort er 1 igen.
    registry::close_card(a.name).expect("close a");
    let h = create_default(dir.path());
    assert_eq!(
        (h.number, h.name.as_str()),
        (1, "card-1"),
        "hul i starten genbruges"
    );

    // Browser-kort deler allokatoren (spec §3): luk d (4), browser-kortet faar 4.
    registry::close_card(d.name).expect("close d");
    let br = registry::create_browser_card(
        None,
        "scope-test".to_string(),
        "https://example.com".to_string(),
        String::new(),
    )
    .expect("create browser card");
    assert_eq!(
        (br.number, br.name.as_str()),
        (4, "card-4"),
        "browser-kort deler nummerserien"
    );
}

// ---------- (c) close draeber pty ----------

#[test]
fn close_card_kills_running_pty_and_removes_card() {
    let _serial = common::serial();
    let dir = tempfile::tempdir().unwrap();
    // Custom long-running kommando (cmd /c ping -t-moensteret).
    let info = registry::create_card(
        dir.path().display().to_string(),
        "claude".to_string(),
        Some(format!("{} /c ping -t 127.0.0.1", cmd_exe())),
    )
    .expect("create_card with custom command");

    // Attach en koerende pty som main.rs' spawn-sti ville goere det —
    // lifecycle-testen ejer spawnet (lib-cratet, ingen AppHandle).
    let handle = registry::card_handle(&info.name).expect("card_handle");
    let (command, cwd) = {
        let card = handle.lock().unwrap();
        let term = card.terminal().expect("terminal card");
        (term.config.command.clone(), term.config.cwd.clone())
    };
    let host = Arc::new(
        PtyHost::spawn(
            PtySpawn {
                cwd,
                command,
                cols: 80,
                rows: 24,
                env_deny_prefixes: vec![],
                env_deny_exact: vec![],
                extra_env: vec![],
            },
            |_: &[u8]| {},
        )
        .expect("spawn ping -t"),
    );
    {
        let mut card = handle.lock().unwrap();
        card.terminal_mut().expect("terminal card").pty = Some(Arc::clone(&host));
    }
    assert!(
        host.try_exit_status().is_none(),
        "ping -t must still be running before close"
    );
    let listed = registry::list_cards();
    let entry = listed
        .iter()
        .find(|c| c.name == info.name)
        .expect("card in list");
    assert!(
        entry.running,
        "list_cards skal vise running=true for et attached pty"
    );

    registry::close_card(info.name.clone()).expect("close_card must kill + remove");

    // Processen er vaek: exit observeres via try_exit_status (ALDRIG EOF-vent).
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if host.try_exit_status().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "child process still alive 10s after close_card"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // Og kortet er ude af registryet.
    assert!(
        registry::card_handle(&info.name).is_err(),
        "handle must be gone"
    );
    assert!(
        !registry::list_cards().iter().any(|c| c.name == info.name),
        "closed card must be gone from list_cards"
    );
}

// ---------- (d) create m. ugyldig cwd -> Err ----------

#[test]
fn create_card_with_missing_cwd_is_rejected() {
    let _serial = common::serial();
    let bogus = r"C:\talminal-no-such-dir-task5\sub";
    let err = registry::create_card(bogus.to_string(), "claude".to_string(), None)
        .expect_err("nonexistent cwd must be rejected");
    assert!(
        err.contains("cwd not found"),
        "beskrivende fejl kraevet, fik: {err}"
    );
    assert!(
        err.contains("talminal-no-such-dir-task5"),
        "fejlen skal naevne stien, fik: {err}"
    );
}

// ---------- (e) resume-binding ----------

#[test]
fn profile_card_resumes_with_profile_resume_command() {
    let _serial = common::serial();
    let dir = tempfile::tempdir().unwrap();
    let (spawn_cmd, resume_cmd) = claude_commands();

    let info = create_default(dir.path());
    let handle = registry::card_handle(&info.name).expect("card_handle");
    {
        let card = handle.lock().unwrap();
        let term = card.terminal().expect("terminal card");
        assert_eq!(
            term.config.command, spawn_cmd,
            "profil-kort spawner profilens command"
        );
        assert_eq!(
            term.config.resume_command, resume_cmd,
            "profil-kort resumer med PROFILENS resume_command (claude --continue)"
        );
    }
    registry::close_card(info.name).expect("close cleanup");
}

#[test]
fn custom_command_card_resumes_with_same_command() {
    let _serial = common::serial();
    let dir = tempfile::tempdir().unwrap();
    let info = registry::create_card(
        dir.path().display().to_string(),
        "claude".to_string(),
        Some("cmd /c echo custom-resume".to_string()),
    )
    .expect("create_card with custom command");
    let handle = registry::card_handle(&info.name).expect("card_handle");
    {
        let card = handle.lock().unwrap();
        let term = card.terminal().expect("terminal card");
        assert_eq!(
            term.config.command,
            vec!["cmd", "/c", "echo", "custom-resume"]
        );
        assert_eq!(
            term.config.resume_command, term.config.command,
            "custom-kort resumer med SAMME command — ingen --continue-semantik"
        );
    }
    registry::close_card(info.name).expect("close cleanup");
}

// ---------- fejlflader (plan-invarianterne) ----------

#[test]
fn close_card_on_unknown_name_is_descriptive_error() {
    let _serial = common::serial();
    let err = registry::close_card("card-999999999".to_string())
        .expect_err("unknown card must be an error");
    assert!(err.contains("no such card"), "fik: {err}");
    assert!(
        err.contains("card-999999999"),
        "fejlen skal naevne navnet, fik: {err}"
    );
}

#[test]
fn create_card_with_unknown_profile_is_rejected() {
    // codex/agent-adapter Task 1: "codex" findes nu — "cursor" er ukendt.
    let _serial = common::serial();
    let dir = tempfile::tempdir().unwrap();
    let err = registry::create_card(dir.path().display().to_string(), "cursor".to_string(), None)
        .expect_err("unknown profile must be rejected");
    assert!(err.contains("unknown profile"), "fik: {err}");
}

// ---------- toml-seed (indtil Task 6 overtager) ----------

#[test]
fn seed_keeps_toml_name_assigns_number_and_rejects_duplicates() {
    let _serial = common::serial();
    let dir = tempfile::tempdir().unwrap();
    let cfg = CardConfig {
        name: "seed-t5-a".to_string(),
        cwd: dir.path().to_path_buf(),
        command: vec!["claude".to_string()],
        resume_command: vec!["claude".to_string(), "--continue".to_string()],
    };
    let info = registry::seed_card(cfg.clone(), "claude").expect("seed_card");
    assert_eq!(info.name, "seed-t5-a", "toml-navnet BEHOLDES (grid-kompat)");
    assert!(
        info.number >= 1,
        "seedede kort faar nummer fra samme taeller"
    );
    assert_eq!(info.profile, "claude");
    assert!(
        registry::list_cards().iter().any(|c| c.name == "seed-t5-a"),
        "seeded card must appear in list_cards (get_cards laeser herfra)"
    );

    // Navne-kollision (korrupt workspace-load-fladen, Task 6-graensefladen).
    let err = registry::seed_card(cfg, "claude").expect_err("duplicate seed must fail");
    assert!(err.contains("duplicate card name"), "fik: {err}");
    assert!(
        err.contains("seed-t5-a"),
        "fejlen skal naevne navnet, fik: {err}"
    );

    registry::close_card("seed-t5-a".to_string()).expect("close cleanup");
}

// ---------- T3: spawn-vejens gates (profil-drevne, ikke custom_command-alene) ----------

#[test]
fn readiness_gate_binds_to_profile_program_stem() {
    // K1-laasen: claude-profil + custom feed-kommando => ingen readiness;
    // codex-profil + codex-kommando => readiness. Testes via den offentlige
    // hjaelper (gaten i spawn_into er selv integrationsdaekket af smoke).
    use talminal_canvas_lib::profiles::readiness_for_command;
    assert!(readiness_for_command("claude", "uv").is_none());
    assert!(readiness_for_command("codex", "codex").is_some());
}
