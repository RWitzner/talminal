// Integrationstests for agent-profil-modulet + env-politik-splittet (Task 2).
//
// Kontrakt (plan Task 2, bindende):
//   - profiles::profile("claude") — CC-only i MVP (laast ejer-beslutning)
//   - resume_command == ["claude", "--continue"] (laast: session-restore)
//   - env-deny: to-lags-semantik (prefix + exact) fra pty.rs' oprindelige
//     DENY-lister — flad exact-match ville tavst laekke OPENAI_*/AWS_*/VERCEL_*
//   - nested-scrub (CLAUDECODE, CLAUDE_CODE_*) er UBETINGET i spawn-stien:
//     anvendes ogsaa med TOMME deny-felter
//
// Env-verifikation via spawn af `cmd /c set`-moensteret som pty_host.rs:
// sentinel-assertions paa ANSI-strippet output; exit via try_exit_status
// (ALDRIG reader-EOF, FUND 11).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use talminal_canvas_lib::profiles::{self, NESTED_SCRUB};
use talminal_canvas_lib::pty::{PtyHost, PtySpawn};

mod common;
use common::{cmd_exe, stripped, wait_exit, wait_for};

// ---------- hjaelpere ----------

/// Spawner `cmd /c set` med de givne deny-lister og opsamler output.
fn spawn_env_dump(
    env_deny_prefixes: Vec<String>,
    env_deny_exact: Vec<String>,
    extra_env: Vec<(String, String)>,
) -> (PtyHost, Arc<Mutex<Vec<u8>>>) {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buf);
    let host = PtyHost::spawn(
        PtySpawn {
            cwd: std::env::temp_dir(),
            command: vec![cmd_exe(), "/c".into(), "set".into()],
            cols: 120,
            rows: 35,
            env_deny_prefixes,
            env_deny_exact,
            extra_env,
        },
        move |bytes| sink.lock().unwrap().extend_from_slice(bytes),
    )
    .expect("spawn");
    (host, buf)
}

// ---------- (a) profil-kontrakten ----------

#[test]
fn claude_profile_contract() {
    let p = profiles::profile("claude").expect("claude profile must exist");
    assert_eq!(p.id, "claude");
    assert_eq!(p.spawn_command, ["claude"]);
    // Laast ejer-beslutning: session-restore = `claude --continue`.
    assert_eq!(p.resume_command, ["claude", "--continue"]);
    // CC beholder FULD deny-liste inkl. ANTHROPIC_API_KEY (laast beslutning).
    assert!(
        p.env_deny_exact.contains(&"ANTHROPIC_API_KEY"),
        "CC profile env_deny must contain ANTHROPIC_API_KEY"
    );
    // To-lags-semantikken: prefix-laget skal daekke OPENAI_/AWS_/VERCEL_-familierne.
    assert!(p.env_deny_prefixes.contains(&"CLAUDE"));
    assert!(p.env_deny_prefixes.contains(&"OPENAI_"));
    assert!(p.env_deny_prefixes.contains(&"VERCEL_"));
    // Write-koreografi (ConPTY-empiri, FUND 7): 300-400 ms-klassen.
    assert_eq!(p.submit_gap_ms, 350);
    // Transcript-roden: ~/.claude/projects/
    let root: PathBuf = (p.transcript_root)();
    assert!(
        root.ends_with(Path::new(".claude/projects")),
        "transcript_root must end with .claude/projects, got: {}",
        root.display()
    );
}

#[test]
fn profile_lookup_is_cc_only() {
    // codex/agent-adapter Task 1 (spec rev 1.2): codex-profilen findes nu.
    assert!(profiles::profile("codex").is_some());
    assert!(profiles::profile("cursor").is_none());
    assert!(profiles::profile("").is_none());
}

#[test]
fn nested_scrub_patterns_are_the_locked_set() {
    assert_eq!(
        NESTED_SCRUB,
        ["CLAUDECODE", "CLAUDE_CODE_*", "CODEX_SANDBOX*"]
    );
}

// ---------- (b) nested-vaern er UBETINGET (tomme deny-felter) ----------

#[test]
fn nested_scrub_applies_with_empty_deny_lists() {
    // Testen saetter SELV nested-markoererne i parent-env FOER spawn, saa
    // asserten altid er skarp — ogsaa uden for en CC-session.
    std::env::set_var("CLAUDECODE", "1");
    std::env::set_var("CLAUDE_CODE_NESTED_PROBE", "must-not-reach-child");
    // Positiv kontrol: en vilkaarlig parent-var uden for begge lag skal arves.
    std::env::set_var("CANVAS_T2_KEEP", "survives");
    let (host, buf) = spawn_env_dump(
        vec![], // TOM deny — nested-vaernet skal virke alligevel
        vec![],
        vec![("TALMINAL_SESSION_ID".into(), "card-nested".into())],
    );
    assert!(
        wait_for(
            &buf,
            "TALMINAL_SESSION_ID=card-nested",
            Duration::from_secs(10)
        ),
        "child env dump not seen; stripped output: {}",
        stripped(&buf)
    );
    assert!(wait_exit(&host, Duration::from_secs(10)).is_some());
    // giv reader-traaden et oejeblik til at draene resten foer fravaers-asserts
    std::thread::sleep(Duration::from_millis(300));
    let text = stripped(&buf);
    for k in ["CLAUDECODE", "CLAUDE_CODE_NESTED_PROBE"] {
        assert!(
            !text.contains(&format!("\n{k}=")),
            "nested guard is unconditional: {k} must be scrubbed even with empty deny lists"
        );
    }
    assert!(
        text.contains("\nCANVAS_T2_KEEP=survives"),
        "scrub must be targeted, not env_clear: CANVAS_T2_KEEP must be inherited"
    );
    host.kill_and_teardown().expect("teardown");
}

// ---------- (c) CC-profilens deny-lister scrubber credentials (to lag) ----------

#[test]
fn cc_profile_deny_lists_scrub_credentials_in_both_layers() {
    // Skarpe asserts: testen saetter selv credentials i parent-env FOER spawn.
    // ANTHROPIC_API_KEY rammes KUN af exact-laget; OPENAI_API_KEY/VERCEL_TOKEN
    // KUN af prefix-laget — testen beviser dermed begge lag.
    std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-must-never-reach-child");
    std::env::set_var("OPENAI_API_KEY", "sk-test-must-never-reach-child");
    std::env::set_var("VERCEL_TOKEN", "vc-test-must-never-reach-child");
    let p = profiles::profile("claude").expect("claude profile");
    let (host, buf) = spawn_env_dump(
        p.env_deny_prefixes.iter().map(|s| s.to_string()).collect(),
        p.env_deny_exact.iter().map(|s| s.to_string()).collect(),
        vec![("TALMINAL_SESSION_ID".into(), "card-deny".into())],
    );
    assert!(
        wait_for(
            &buf,
            "TALMINAL_SESSION_ID=card-deny",
            Duration::from_secs(10)
        ),
        "child env dump not seen; stripped output: {}",
        stripped(&buf)
    );
    assert!(wait_exit(&host, Duration::from_secs(10)).is_some());
    std::thread::sleep(Duration::from_millis(300));
    let text = stripped(&buf);
    for k in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "VERCEL_TOKEN"] {
        assert!(
            !text.contains(&format!("\n{k}=")),
            "denylisted {k} must be scrubbed from child env"
        );
    }
    host.kill_and_teardown().expect("teardown");
}
