// Task 11 — restore-on-launch: ren plan-logik (lib-cratet, ingen app-koersel).
//
// Testcases (planens Step 1 (a)-(e)):
//   (a) to kort samme cwd -> nyeste Resume, andet FreshSharedCwd
//   (b) tre cwd'er et kort hver -> tre Resume
//   (c) manglende last_active_at i delt gruppe taber (og bliver Fresh —
//       laast beslutning 9: None -> Fresh UANSET gruppering)
//   (d) case-/separator-varianter af samme sti grupperes (kanonikalisering);
//       baade eksisterende sti (canonicalize) og doed sti (tekstuel fallback)
//   (e) kort m. last_active_at == None -> Fresh, ogsaa alene i sin cwd
//   (+) CardInfo baerer restore_action: None ved create, sat via
//       registry::set_restore_action (app-start-stien) og synlig i list_cards
//
// restore_plan er REN logik over WorkspaceCard-slices — ingen registry-/env-
// afhaengighed, saa (a)-(e) er parallel-robuste uden worker-processer.

use talminal_canvas_lib::registry;
use talminal_canvas_lib::restore::{restore_plan, RestoreAction};
use talminal_canvas_lib::workspace::WorkspaceCard;

// ---------- hjaelpere ----------

fn card(number: u32, name: &str, cwd: &str, last_active_at: Option<&str>) -> WorkspaceCard {
    WorkspaceCard {
        number,
        name: name.to_string(),
        cwd: cwd.to_string(),
        profile: "claude".to_string(),
        command: None,
        x: 0.0,
        y: 0.0,
        w: 960.0,
        h: 640.0,
        last_active_at: last_active_at.map(str::to_string),
    }
}

/// T8 (B1): samme som `card`, men med en eksplicit profil — bruges til at
/// bevise at gruppenoeglen er (cwd, profile), ikke cwd alene.
fn card_with_profile(
    number: u32,
    name: &str,
    cwd: &str,
    profile: &str,
    last_active_at: Option<&str>,
) -> WorkspaceCard {
    WorkspaceCard {
        profile: profile.to_string(),
        ..card(number, name, cwd, last_active_at)
    }
}

fn action_of(plan: &[(String, RestoreAction)], name: &str) -> RestoreAction {
    plan.iter()
        .find(|(n, _)| n == name)
        .map(|(_, a)| *a)
        .unwrap_or_else(|| panic!("card '{name}' missing from plan"))
}

// ---------- (a) delt cwd: nyeste vinder ----------

#[test]
fn a_shared_cwd_newest_resumes_other_fresh_shared() {
    let cards = vec![
        card(
            1,
            "card-1",
            r"C:\t11-dead\proj-a",
            Some("2026-07-17T10:00:00.000Z"),
        ),
        card(
            2,
            "card-2",
            r"C:\t11-dead\proj-a",
            Some("2026-07-17T11:30:00.000Z"),
        ),
    ];
    let plan = restore_plan(&cards);
    assert_eq!(plan.len(), 2, "plan skal daekke ALLE kort");
    assert_eq!(action_of(&plan, "card-2"), RestoreAction::Resume);
    assert_eq!(action_of(&plan, "card-1"), RestoreAction::FreshSharedCwd);
}

// ---------- (b) tre cwd'er, et kort hver ----------

#[test]
fn b_distinct_cwds_all_resume() {
    let cards = vec![
        card(
            1,
            "card-1",
            r"C:\t11-dead\proj-a",
            Some("2026-07-17T08:00:00.000Z"),
        ),
        card(
            2,
            "card-2",
            r"C:\t11-dead\proj-b",
            Some("2026-07-17T09:00:00.000Z"),
        ),
        card(
            3,
            "card-3",
            r"C:\t11-dead\proj-c",
            Some("2026-07-17T10:00:00.000Z"),
        ),
    ];
    let plan = restore_plan(&cards);
    assert_eq!(plan.len(), 3);
    for name in ["card-1", "card-2", "card-3"] {
        assert_eq!(action_of(&plan, name), RestoreAction::Resume, "{name}");
    }
}

// ---------- (c) None i delt gruppe taber ----------

#[test]
fn c_none_last_active_in_shared_group_loses() {
    let cards = vec![
        card(1, "card-1", r"C:\t11-dead\proj-d", None),
        card(
            2,
            "card-2",
            r"C:\t11-dead\proj-d",
            Some("2026-07-17T09:00:00.000Z"),
        ),
    ];
    let plan = restore_plan(&cards);
    // Kortet MED session vinder gruppen — selv med aeldre-end-alt timestamp.
    assert_eq!(action_of(&plan, "card-2"), RestoreAction::Resume);
    // Laast beslutning 9: None -> Fresh UANSET gruppering (aldrig
    // FreshSharedCwd, aldrig --continue i en cwd uden session).
    assert_eq!(action_of(&plan, "card-1"), RestoreAction::Fresh);
}

// ---------- (d) kanonikalisering ----------

#[test]
fn d_case_and_separator_variants_of_existing_path_group() {
    // Eksisterende sti: std::fs::canonicalize oploeser baade case- og
    // separator-varianter til EN form (Windows-fs er case-insensitivt).
    let dir = tempfile::tempdir().expect("tempdir");
    let real = dir.path().join("Proj-X");
    std::fs::create_dir(&real).expect("mkdir");
    let real_str = real.to_string_lossy().to_string();
    let variant = real_str.to_uppercase().replace('\\', "/");
    assert_ne!(real_str, variant, "testen kraever en reel variant");
    let cards = vec![
        card(1, "card-1", &real_str, Some("2026-07-17T10:00:00.000Z")),
        card(2, "card-2", &variant, Some("2026-07-17T11:00:00.000Z")),
    ];
    let plan = restore_plan(&cards);
    assert_eq!(action_of(&plan, "card-2"), RestoreAction::Resume);
    assert_eq!(
        action_of(&plan, "card-1"),
        RestoreAction::FreshSharedCwd,
        "varianterne skal gruppere som SAMME cwd"
    );
}

#[test]
fn d_dead_path_variants_group_via_fallback() {
    // Doed sti (canonicalize fejler): den dokumenterede tekstuelle fallback
    // ('/' -> '\', trailing-separator-trim, lowercase) skal stadig gruppere.
    let cards = vec![
        card(
            1,
            "card-1",
            r"C:\T11-Definitely\Not\Here-42",
            Some("2026-07-17T10:00:00.000Z"),
        ),
        card(
            2,
            "card-2",
            "c:/t11-definitely/not/HERE-42/",
            Some("2026-07-17T11:00:00.000Z"),
        ),
    ];
    let plan = restore_plan(&cards);
    assert_eq!(action_of(&plan, "card-2"), RestoreAction::Resume);
    assert_eq!(action_of(&plan, "card-1"), RestoreAction::FreshSharedCwd);
}

// ---------- (e) None alene i sin cwd ----------

#[test]
fn e_none_last_active_alone_is_fresh() {
    let cards = vec![card(1, "card-1", r"C:\t11-dead\proj-e", None)];
    let plan = restore_plan(&cards);
    // Aldrig Resume uden session — CC ville fejle ind i exit-overlayet.
    assert_eq!(action_of(&plan, "card-1"), RestoreAction::Fresh);
}

// ---------- (f) delt cwd, forskellig profil: begge resumer (T8/B1) ----------

#[test]
fn f_shared_cwd_different_profile_both_resume() {
    // Et claude- og et codex-kort i SAMME cwd konkurrerer IKKE om samme
    // resume-slot: gruppenoeglen er (cwd, profile), ikke cwd alene. Codex-
    // kortet har det AELDSTE timestamp for at bevise at det ikke bare vinder
    // fordi det taber til claude-kortet i en faelles gruppe.
    let cards = vec![
        card_with_profile(
            1,
            "card-1",
            r"C:\t11-dead\proj-f",
            "claude",
            Some("2026-07-17T10:00:00.000Z"),
        ),
        card_with_profile(
            2,
            "card-2",
            r"C:\t11-dead\proj-f",
            "codex",
            Some("2026-07-17T09:00:00.000Z"),
        ),
    ];
    let plan = restore_plan(&cards);
    assert_eq!(plan.len(), 2);
    assert_eq!(
        action_of(&plan, "card-1"),
        RestoreAction::Resume,
        "claude-kortet er alene i sin (cwd, claude)-gruppe"
    );
    assert_eq!(
        action_of(&plan, "card-2"),
        RestoreAction::Resume,
        "codex-kortet konkurrerer IKKE med claude-kortet om cwd'ens slot"
    );
}

// ---------- CardInfo.restore_action (registry-fladen) ----------

#[test]
fn card_info_carries_restore_action_set_at_startup() {
    // Registryet er en global singleton, men denne testbinar er sin egen
    // proces og ingen andre tests her roerer registryet — ingen worker noedvendig.
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();
    let info = registry::create_card(cwd, "claude".to_string(), None).expect("create_card");
    assert_eq!(info.restore_action, None, "kort foedes uden badge");
    registry::set_restore_action(&info.name, "fresh_shared_cwd").expect("set_restore_action");
    let listed = registry::list_cards()
        .into_iter()
        .find(|c| c.name == info.name)
        .expect("kortet skal staa i list_cards");
    assert_eq!(listed.restore_action.as_deref(), Some("fresh_shared_cwd"));
    // Ukendt kort: samme fejlflade som resten af registryet.
    let err = registry::set_restore_action("no-such-card", "resume").unwrap_err();
    assert!(err.contains("unknown card"), "fik: {err}");
}
