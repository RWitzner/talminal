//! Task 11: [master]-entry i cards.toml — navnet er tvunget til "master"
//! (Task 7's parse), resume_command er ALTID command (en feed-tail har ingen
//! --continue), resolve_card lader master skygge et manuelt konstrueret
//! [[card]] med samme navn, og de mekaniske read-only-guards afviser "master"
//! i write_pty-/resume-vejen (main.rs kalder dem som FØRSTE linje).
//! Ejer-beslutning 1: master-kort v0 = read-only `persona feed --follow`.
//!
//! Task 3: master-kortet er ude af MVP-pathen — hele filen er supervision-only
//! (default-state-kontrakten er testet i tests/master_off.rs).
#![cfg(feature = "supervision")]

use std::fs;
use std::path::PathBuf;

use talminal_canvas_lib::cards::{
    guard_resume_control, guard_write_pty, load_cards, resolve_card, CardConfig, CardsFile,
    MASTER_NAME,
};

fn write_toml(tag: &str, content: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "talminal-cards-{}-{}.toml",
        std::process::id(),
        tag
    ));
    fs::write(&path, content).unwrap();
    path
}

const MASTER_TOML: &str = r#"
[master]
command = ["uv", "run", "--directory", "C:/projekter/demo/controller", "persona", "feed", "--follow"]
cwd = "C:/projekter/demo"
resume_command = ["ignored", "on", "purpose"]

[[card]]
name = "a"
cwd = "C:/code/proj-a"
"#;

#[test]
fn master_name_is_forced_and_resume_is_command() {
    let path = write_toml("forced", MASTER_TOML);
    let file = load_cards(&path).unwrap();
    fs::remove_file(&path).ok();
    let master = file.master.as_ref().expect("[master] skal parse til Some");
    assert_eq!(master.name, MASTER_NAME);
    assert_eq!(master.command[..2], ["uv".to_string(), "run".to_string()]);
    assert_eq!(master.command.last().unwrap(), "--follow");
    // Feed-tail har ingen --continue: resume_command er ALTID command,
    // også naar TOML'en saetter et eksplicit resume_command (ignoreres).
    assert_eq!(master.resume_command, master.command);
    assert_eq!(file.cards.len(), 1);
}

#[test]
fn missing_master_table_is_none() {
    let path = write_toml("none", "[[card]]\nname = \"a\"\ncwd = \"C:/code/proj-a\"\n");
    let file = load_cards(&path).unwrap();
    fs::remove_file(&path).ok();
    assert!(file.master.is_none());
}

#[test]
fn resolve_card_master_shadows_card_named_master() {
    // CardsFile konstrueres MANUELT udenom load_cards: Task 7's validering
    // (master_name_reserved_for_master_card) afviser [[card]] name="master"
    // allerede ved parse, saa kollisionen kan ikke opstaa ad fil-vejen —
    // resolve_card er defense-in-depth for haandbyggede CardsFile-vaerdier.
    let master = CardConfig {
        name: MASTER_NAME.to_string(),
        cwd: PathBuf::from("C:/projekter/demo"),
        command: vec![
            "persona".to_string(),
            "feed".to_string(),
            "--follow".to_string(),
        ],
        resume_command: vec![
            "persona".to_string(),
            "feed".to_string(),
            "--follow".to_string(),
        ],
    };
    let impostor = CardConfig {
        name: MASTER_NAME.to_string(),
        cwd: PathBuf::from("C:/code/impostor"),
        command: vec!["claude".to_string()],
        resume_command: vec!["claude".to_string(), "--continue".to_string()],
    };
    let worker = CardConfig {
        name: "a".to_string(),
        cwd: PathBuf::from("C:/code/proj-a"),
        command: vec!["claude".to_string()],
        resume_command: vec!["claude".to_string(), "--continue".to_string()],
    };
    let file = CardsFile {
        master: Some(master),
        cards: vec![impostor, worker],
    };
    assert_eq!(
        resolve_card(&file, MASTER_NAME).unwrap().cwd,
        PathBuf::from("C:/projekter/demo")
    );
    assert_eq!(resolve_card(&file, "a").unwrap().name, "a");
    assert!(resolve_card(&file, "ukendt").is_none());
}

#[test]
fn guard_write_pty_rejects_master_only() {
    // Regel 1's mekaniske backend-haandhaevelse: main.rs kalder guarden som
    // FOERSTE linje i write_pty — en frontend-fejl kan aldrig skrive bytes
    // eller udloese pause-signalfiler for master.
    assert_eq!(
        guard_write_pty(MASTER_NAME),
        Err("master_readonly".to_string())
    );
    assert_eq!(guard_write_pty("a"), Ok(()));
}

#[test]
fn guard_resume_control_rejects_master_only() {
    assert_eq!(
        guard_resume_control(MASTER_NAME),
        Err("master_has_no_pause".to_string())
    );
    assert_eq!(guard_resume_control("a"), Ok(()));
}
