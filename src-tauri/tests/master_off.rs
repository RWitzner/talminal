//! Task 3: master-kortet er ude af MVP-pathen (låst ejer-beslutning).
//! Default-state-kontrakten: cards.toml MED en [master]-sektion indlæses uden
//! fejl — sektionen IGNORERES (logline i load_cards), og der findes ingen
//! master i kortlisten. Guard-vejene (write_pty/resume_card_control i main.rs)
//! afviser aldrig noget kort — heller ikke navnet "master".
//! Supervision-adfærden (master-parse + read-only-guards) er testet i
//! tests/cards_master.rs, som kun kører med `--features supervision`.
#![cfg(not(feature = "supervision"))]

use std::fs;
use std::path::PathBuf;

use talminal_canvas_lib::cards::{guard_resume_control, guard_write_pty, load_cards};

fn write_toml(tag: &str, content: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "talminal-master-off-{}-{}.toml",
        std::process::id(),
        tag
    ));
    fs::write(&path, content).unwrap();
    path
}

/// Velformet [master] (som i dogfood-cards.toml) + ét normalt kort.
const MASTER_TOML: &str = r#"
[master]
command = ["uv", "run", "--directory", "C:/projekter/demo/controller", "persona", "feed", "--follow"]
cwd = "C:/projekter/demo"

[[card]]
name = "a"
cwd = "C:/code/proj-a"
"#;

#[test]
fn master_section_is_ignored_without_error() {
    // (a) given cards.toml MED [master], when load, then ingen fejl og ingen
    // master i kortlisten — kun [[card]]-entries kommer med.
    let path = write_toml("ignored", MASTER_TOML);
    let file = load_cards(&path).unwrap();
    fs::remove_file(&path).ok();
    assert_eq!(file.cards.len(), 1, "kun [[card]]-entries i kortlisten");
    assert_eq!(file.cards[0].name, "a");
    assert!(
        file.cards.iter().all(|c| c.name != "master"),
        "ingen master i kortlisten i default-state"
    );
}

#[test]
fn invalid_master_section_is_ignored_too() {
    // [master] uden command er en Invalid-fejl under supervision (F21) — i
    // default-state ignoreres sektionen WHOLESALE og må ALDRIG knække loadet.
    let path = write_toml("invalid", "[master]\ncwd = \"C:/projekter/demo\"\n");
    let res = load_cards(&path);
    fs::remove_file(&path).ok();
    let file = res.expect("[master] uden command må ikke fejle i default-state");
    assert!(file.cards.is_empty());
}

#[test]
fn write_guards_are_noops_in_default_state() {
    // (b) write_pty-vejen har ingen guard-fejl: main.rs kalder guard_write_pty
    // som første linje, og i default-state er den en no-op for ALLE navne —
    // "master" er ikke et reserveret/read-only kort i MVP-pathen.
    assert_eq!(guard_write_pty("a"), Ok(()));
    assert_eq!(guard_write_pty("master"), Ok(()));
    assert_eq!(guard_resume_control("a"), Ok(()));
    assert_eq!(guard_resume_control("master"), Ok(()));
}
