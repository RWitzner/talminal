use std::path::Path;
use std::time::Duration;

use talminal_canvas_lib::transcripts::{
    project_slug, read_transcript_tail_for_profile, read_transcript_tail_for_profile_with_roots,
    read_transcript_tail_from_projects, TranscriptEntry,
};

fn write_jsonl(path: &Path, lines: &[&str]) {
    std::fs::write(path, format!("{}\n", lines.join("\n"))).expect("write fixture jsonl");
}

#[test]
fn project_slug_replaces_separators_and_drive_colon_like_claude_code() {
    // Testen kraevede tidligere ogsaa at %USERPROFILE%\.claude\projects\... laa
    // paa disken. Den var hverken #[ignore]d eller cfg-gated, saa den bestod kun
    // fordi ejerens egen maskine tilfaeldigvis havde mappen — enhver anden der
    // klonede repoet fik en roed suite ved foerste `cargo test`. Slug-reglen er
    // ren strengtransformation og bevises som saadan; koblingen til CC's
    // virkelige mappenavne holdes af doc-kommentaren paa project_slug.
    let slug = project_slug(Path::new(r"C:\projekter\demo"));
    assert_eq!(slug, "C--projekter-demo");
}

#[test]
fn newest_jsonl_returns_last_user_and_assistant_text_without_tool_noise() {
    let temp = tempfile::tempdir().unwrap();
    let projects = temp.path().join("projects");
    let cwd = Path::new(r"C:\projekter\demo");
    let project = projects.join(project_slug(cwd));
    std::fs::create_dir_all(&project).unwrap();

    write_jsonl(
        &project.join("older.jsonl"),
        &[
            r#"{"type":"user","message":{"role":"user","content":"gammel"},"timestamp":"2026-07-01T08:00:00Z"}"#,
        ],
    );
    std::thread::sleep(Duration::from_millis(20));
    write_jsonl(
        &project.join("newer.jsonl"),
        &[
            r#"{"type":"user","message":{"role":"user","content":"Kør testene"},"timestamp":"2026-07-18T10:00:00Z"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"skjult"},{"type":"text","text":"Jeg kører dem nu"},{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]},"timestamp":"2026-07-18T10:00:01Z"}"#,
            r#"{"type":"user","message":{"role":"tool","content":"156 passed"},"timestamp":"2026-07-18T10:00:02Z"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Alle tests er grønne"}]},"timestamp":"2026-07-18T10:00:03Z"}"#,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"intern meta"},"timestamp":"2026-07-18T10:00:04Z"}"#,
        ],
    );

    let tail = read_transcript_tail_from_projects(&projects, cwd, 2).expect("tail");

    assert!(tail.per_directory);
    assert_eq!(
        tail.entries,
        vec![
            TranscriptEntry {
                role: "assistant".into(),
                text: "Jeg kører dem nu".into(),
                ts: "2026-07-18T10:00:01Z".into(),
            },
            TranscriptEntry {
                role: "assistant".into(),
                text: "Alle tests er grønne".into(),
                ts: "2026-07-18T10:00:03Z".into(),
            },
        ]
    );
}

#[test]
fn missing_or_empty_project_directory_is_ok_empty_and_per_directory() {
    let temp = tempfile::tempdir().unwrap();
    let projects = temp.path().join("projects");
    std::fs::create_dir_all(&projects).unwrap();
    let cwd = Path::new(r"C:\projekter\does-not-exist");

    let missing = read_transcript_tail_from_projects(&projects, cwd, 12).expect("missing");
    assert!(missing.per_directory);
    assert!(missing.entries.is_empty());

    std::fs::create_dir_all(projects.join(project_slug(cwd))).unwrap();
    let empty = read_transcript_tail_from_projects(&projects, cwd, 12).expect("empty");
    assert!(empty.per_directory);
    assert!(empty.entries.is_empty());
}

// ---------- T8 (B4): profil-bevidst rod — codex maa ALDRIG laese CC's rod ----------

#[test]
fn codex_card_in_claude_cwd_returns_empty_tail() {
    // Samme cwd har en REEL claude-session, men kortets profil er "codex" og
    // den injicerede codex-rod findes ikke -> tom hale, uanset hvad claude-
    // roden indeholder (B4-laasen: ingen claude-fallback).
    let temp = tempfile::tempdir().unwrap();
    let claude_root = temp.path().join("claude-projects");
    let codex_root = temp.path().join("codex-sessions"); // findes ikke
    let cwd = Path::new(r"C:\projekter\demo");
    let project = claude_root.join(project_slug(cwd));
    std::fs::create_dir_all(&project).unwrap();
    write_jsonl(
        &project.join("session.jsonl"),
        &[r#"{"type":"user","message":{"role":"user","content":"cc-besked"},"timestamp":"now"}"#],
    );

    let tail = read_transcript_tail_for_profile_with_roots(
        "codex",
        &cwd.to_string_lossy(),
        5,
        &claude_root,
        &codex_root,
    )
    .expect("tail");
    assert!(
        tail.entries.is_empty(),
        "codex-kortet maa ikke se claude-roden"
    );
    assert!(tail.per_directory);
}

#[test]
fn claude_card_in_shared_cwd_reads_claude_root() {
    // Positiv-modstykket: samme opsaetning, men profilen ER "claude" -> den
    // rigtige besked laeses fra den injicerede claude-rod.
    let temp = tempfile::tempdir().unwrap();
    let claude_root = temp.path().join("claude-projects");
    let codex_root = temp.path().join("codex-sessions");
    let cwd = Path::new(r"C:\projekter\demo");
    let project = claude_root.join(project_slug(cwd));
    std::fs::create_dir_all(&project).unwrap();
    write_jsonl(
        &project.join("session.jsonl"),
        &[r#"{"type":"user","message":{"role":"user","content":"cc-besked"},"timestamp":"now"}"#],
    );

    let tail = read_transcript_tail_for_profile_with_roots(
        "claude",
        &cwd.to_string_lossy(),
        5,
        &claude_root,
        &codex_root,
    )
    .expect("tail");
    assert_eq!(tail.entries.len(), 1);
    assert_eq!(tail.entries[0].text, "cc-besked");
}

#[test]
fn unknown_profile_in_with_roots_returns_empty_tail_never_claude_fallback() {
    // GPT-review-kravet (spec): en ukendt profil maa ALDRIG laese CC's
    // transcripts — ogsaa naar claude-roden faktisk har data for cwd'en.
    let temp = tempfile::tempdir().unwrap();
    let claude_root = temp.path().join("claude-projects");
    let codex_root = temp.path().join("codex-sessions");
    let cwd = Path::new(r"C:\projekter\demo");
    let project = claude_root.join(project_slug(cwd));
    std::fs::create_dir_all(&project).unwrap();
    write_jsonl(
        &project.join("session.jsonl"),
        &[r#"{"type":"user","message":{"role":"user","content":"cc-besked"},"timestamp":"now"}"#],
    );

    let tail = read_transcript_tail_for_profile_with_roots(
        "cursor",
        &cwd.to_string_lossy(),
        5,
        &claude_root,
        &codex_root,
    )
    .expect("tail");
    assert!(tail.entries.is_empty());
    assert!(tail.per_directory);
}

#[test]
fn read_transcript_tail_for_profile_unknown_profile_is_empty_via_production_routing() {
    // Produktions-indgangen (rigtige CLAUDE_CONFIG_DIR/USERPROFILE-opslag) —
    // beviser routing-laget alene, uafhaengigt af filsystemets indhold.
    let cwd = Path::new(r"C:\projekter\demo");
    let tail = read_transcript_tail_for_profile("not-a-real-agent", &cwd.to_string_lossy(), 5)
        .expect("tail");
    assert!(tail.entries.is_empty());
    assert!(tail.per_directory);
}

#[test]
fn zero_max_entries_returns_empty_without_reading_tool_noise() {
    let temp = tempfile::tempdir().unwrap();
    let projects = temp.path().join("projects");
    let cwd = Path::new(r"C:\repo");
    let project = projects.join(project_slug(cwd));
    std::fs::create_dir_all(&project).unwrap();
    write_jsonl(
        &project.join("session.jsonl"),
        &[r#"{"type":"user","message":{"role":"user","content":"hej"},"timestamp":"now"}"#],
    );

    let tail = read_transcript_tail_from_projects(&projects, cwd, 0).expect("tail");
    assert!(tail.entries.is_empty());
    assert!(tail.per_directory);
}
