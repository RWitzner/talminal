//! Agent-profil-modul + env-politik-split (Task 2) — udvidet med codex-
//! profilen i codex/agent-adapter-planens Task 1 (spec rev 1.2).
//!
//! Laaste ejer-beslutninger (plan/spec §2, maa ikke relitigeres):
//! - kun claude- og codex-profilerne findes; øvrige agenter ("cursor" mv.)
//!   forbliver ukendte indtil deres data-udvidelse besluttes.
//! - session-restore = `claude --continue` (et-resumbart-kort-pr.-cwd).
//! - credential-deny-liste per agent-profil; CC beholder FULD liste inkl.
//!   `ANTHROPIC_API_KEY`.
//! - nested-env-scrub (`CLAUDECODE`, `CLAUDE_CODE_*`) er UBETINGET og bor i
//!   spawn-stien (pty.rs) — uafhaengigt af profilens deny-felter.
//!
//! Journalfoert eksklusion (plan Task 2): spec §4.1's auto-reply-katalog-
//! referencer udelades — kataloget er terminal-protokol-niveau og bor i
//! frontend-modulet `terminalReply.ts` (agent-agnostisk i praksis).

use std::path::PathBuf;

/// En agent-profil: kommandoer + env-politik + submit-/transcript-parametre.
/// Bindende interface (plan Task 2) — senere tasks consumer felterne:
/// registry/restore (spawn_command/resume_command), submit.rs (submit_gap_ms),
/// transcripts.rs (transcript_root).
pub struct AgentProfile {
    pub id: &'static str,
    pub spawn_command: &'static [&'static str],
    pub resume_command: &'static [&'static str],
    /// Prefix-laget af deny-listen (pty.rs' oprindelige DENY_PREFIXES —
    /// prefix-match!). To-lags-semantikken (prefix + exact) er bindende:
    /// flad exact-match ville tavst laekke OPENAI_*/AWS_*/VERCEL_*.
    pub env_deny_prefixes: &'static [&'static str],
    /// Exact-laget af deny-listen (pty.rs' oprindelige DENY_EXACT).
    pub env_deny_exact: &'static [&'static str],
    /// Gab mellem tekst-write og \r-write i submit-koreografien
    /// (ConPTY-empiri FUND 7: 300-400 ms-klassen).
    pub submit_gap_ms: u64,
    /// Rod for agentens transcript-filer (CC: `~/.claude/projects/`).
    pub transcript_root: fn() -> PathBuf,
    /// FSM-readiness-spec — `None` hvis profilen ikke understoetter readiness-
    /// detektion (spec rev 1.2 §1.1).
    pub readiness: Option<&'static ReadinessSpec>,
    /// MCP-injektions-strategi (spec §1.2; vej i spike-verificeret).
    pub mcp: McpInjection,
    /// Fallback-oploesning naar spawn_command[0] ikke findes paa PATH.
    pub exe_fallback: Option<fn() -> Option<PathBuf>>,
    /// Byte-moenstre der betyder "agenten venter paa et svar" — permission- og
    /// confirm-prompts. Tom liste er lovligt (saa gaelder kun stilheds-reglen).
    /// Laeses af workspaces::attention (opmaerksomheds-prikken).
    pub attention_patterns: &'static [&'static [u8]],
}

/// FSM-markører som profil-data (spec rev 1.2 §1.1; spike-tabellen er bindende).
pub struct ReadinessSpec {
    pub require_alt_screen: bool,
    pub live_prompts: &'static [&'static [u8]],
    pub require_column_three: bool,
}

/// MCP-injektions-strategi pr. profil (spec §1.2; vej i spike-verificeret).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpInjection {
    ClaudeFlags,
    CodexOverrides,
}

pub static CLAUDE_READINESS: ReadinessSpec = ReadinessSpec {
    require_alt_screen: true,
    live_prompts: &[b"\xE2\x9D\xAF\xC2\xA0", b">\xC2\xA0"], // ❯+NBSP / >+NBSP
    require_column_three: true,
};

/// codex-cli 0.145.0 (spike 2026-07-24): inline-TUI (INGEN alt-screen);
/// live-composer = ESC[1m + CRLF + › (dim-varianten ESC[2m er shutdown).
pub static CODEX_READINESS: ReadinessSpec = ReadinessSpec {
    require_alt_screen: false,
    live_prompts: &[b"\x1b[1m\r\n\xE2\x80\xBA"],
    require_column_three: true,
};

/// Nested-vaern — ALTID anvendt i spawn-stien (pty.rs), uafhaengigt af
/// profilens deny-felter: et kort-CC maa aldrig se sig selv som nested
/// child-session. `*`-suffix betyder prefix-match; ellers exact-match.
pub const NESTED_SCRUB: &[&str] = &["CLAUDECODE", "CLAUDE_CODE_*", "CODEX_SANDBOX*"];

/// Matcher et env-var-navn mod NESTED_SCRUB-moenstrene.
pub fn nested_scrub_matches(name: &str) -> bool {
    NESTED_SCRUB.iter().any(|pat| match pat.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *pat,
    })
}

/// CC's transcript-rod: `~/.claude/projects/` (Windows: %USERPROFILE%).
/// `CLAUDE_CONFIG_DIR` respekteres FOERST — logikken flyttede hertil fra
/// transcripts.rs' gamle `claude_projects_dir()` ved Task 8's profil-
/// bevidste transcript-rod, saa CC-vejens observerbare adfaerd forbliver
/// bit-identisk (det tidligere DØDE felt wires nu reelt).
fn claude_transcript_root() -> PathBuf {
    if let Some(config) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(config).join("projects");
    }
    // `.unwrap_or_default()` er et BEVIDST valg, ikke en overset detalje: Windows
    // saetter altid USERPROFILE, saa denne gren rammes reelt aldrig i praksis —
    // og `transcript_root`s frosne `fn() -> PathBuf`-signatur (Task 1) tillader
    // ikke at propagere en `Result` her alligevel (Task 8 resolution-3).
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".claude")
        .join("projects")
}

/// Claude Code-profilen. Deny-listerne er pty.rs' oprindelige
/// DENY_PREFIXES/DENY_EXACT VERBATIM (fix F5, spec §3 "minimalt miljoe" —
/// v0 = deny-liste; fuldt minimalt env = M2.5).
static CLAUDE: AgentProfile = AgentProfile {
    id: "claude",
    spawn_command: &["claude"],
    resume_command: &["claude", "--continue"],
    env_deny_prefixes: &[
        "CLAUDE",
        "OPENAI_",
        "AWS_ACCESS",
        "AWS_SECRET",
        "AWS_SESSION",
        "VERCEL_",
    ],
    env_deny_exact: &[
        "ANTHROPIC_API_KEY",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
    ],
    submit_gap_ms: 350,
    transcript_root: claude_transcript_root,
    readiness: Some(&CLAUDE_READINESS),
    mcp: McpInjection::ClaudeFlags,
    exe_fallback: None,
    attention_patterns: &[b"Do you want to proceed?", b"Do you want to make this edit"],
};

/// Codex-cli's transcript-rod: `%CODEX_HOME%/sessions` (default `~/.codex/sessions`).
fn codex_transcript_root() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".codex")
        })
        .join("sessions")
}

/// npm-vendor-fallback (spike §5.6): PATH har typisk kun codex.ps1/.cmd-shims.
/// BEVIDST begraenset til `%APPDATA%\npm`-layoutet i v1 (fallback-succes-vejen
/// er miljoeafhaengig og daekkes af operatoer-roegtesten i T10).
fn codex_exe_fallback() -> Option<PathBuf> {
    let vendor = PathBuf::from(std::env::var_os("APPDATA")?)
        .join("npm")
        .join("node_modules")
        .join("@openai")
        .join("codex")
        .join("node_modules")
        .join("@openai")
        .join("codex-win32-x64")
        .join("vendor");
    let entries = std::fs::read_dir(&vendor).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join("bin").join("codex.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Codex-profilen (spec rev 1.2). Symmetrisk med CC inkl. OPENAI_ (spec §1:
/// kort-auth KUN via `codex login`/auth.json — spike §5.5 bekraeftede at
/// basemiljoeet ikke behoever OPENAI_*).
static CODEX: AgentProfile = AgentProfile {
    id: "codex",
    spawn_command: &["codex"],
    resume_command: &["codex", "resume", "--last"],
    env_deny_prefixes: &[
        "CLAUDE",
        "OPENAI_",
        "AWS_ACCESS",
        "AWS_SECRET",
        "AWS_SESSION",
        "VERCEL_",
    ],
    env_deny_exact: &[
        "ANTHROPIC_API_KEY",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
    ],
    submit_gap_ms: 350,
    transcript_root: codex_transcript_root,
    readiness: Some(&CODEX_READINESS),
    mcp: McpInjection::CodexOverrides,
    exe_fallback: Some(codex_exe_fallback),
    attention_patterns: &[b"Allow command?", b"[y/n]"],
};

/// Slaar en profil op pr. id. Kender nu `"claude"` OG `"codex"` (spec rev 1.2).
pub fn profile(id: &str) -> Option<&'static AgentProfile> {
    match id {
        "claude" => Some(&CLAUDE),
        "codex" => Some(&CODEX),
        _ => None,
    }
}

/// Readiness kun naar profilen HAR en spec OG den faktiske executable er
/// profilens egen (K1: supervision-masterens feed-kommando ekskluderes af
/// stem-checket — custom_command=false er IKKE nok, se main.rs:335-337).
pub fn readiness_for_command(profile_id: &str, program: &str) -> Option<&'static ReadinessSpec> {
    let prof = profile(profile_id)?;
    let spec = prof.readiness?;
    let expected = std::path::Path::new(prof.spawn_command[0])
        .file_stem()?
        .to_ascii_lowercase();
    let actual = std::path::Path::new(program)
        .file_stem()?
        .to_ascii_lowercase();
    (actual == expected).then_some(spec)
}

/// Windows-opløsning (spike §5.6): PATH-søg `{program}.exe` — findes den,
/// returneres INPUTTET uændret (CC-vejen forbliver bit-identisk). Ellers
/// profilens fallback (npm-vendor-exe). ALDRIG cmd /c.
pub fn resolve_spawn_program(prof: &AgentProfile) -> Result<String, String> {
    let program = prof.spawn_command[0];
    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join(format!("{program}.exe")).is_file())
    });
    if on_path {
        return Ok(program.to_string());
    }
    if let Some(fallback) = prof.exe_fallback {
        if let Some(exe) = fallback() {
            return Ok(exe.display().to_string());
        }
    }
    Err(format!(
        "{program}.exe blev ikke fundet paa PATH (er {} installeret?)",
        prof.id
    ))
}

#[cfg(test)]
mod tests {
    use super::nested_scrub_matches;
    use super::AgentProfile;
    use std::sync::Mutex;

    // T8-review-fund: env-mutation er process-global og cargo test koerer
    // unit-tests multi-traadet i EN binary — spejler project.rs'/tests/
    // instance.rs' etablerede moenster (samme fil-lease-modul har ikke egen
    // ENV_LOCK endnu, saa den indfoeres her for CLAUDE_CONFIG_DIR-testen og
    // enhver fremtidig CODEX_HOME-soeskende-test).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn nested_scrub_matches_exact_and_wildcard_only() {
        assert!(nested_scrub_matches("CLAUDECODE"));
        assert!(nested_scrub_matches("CLAUDE_CODE_ENTRYPOINT"));
        assert!(nested_scrub_matches("CLAUDE_CODE_")); // prefix-graensetilfaelde
        assert!(!nested_scrub_matches("CLAUDECODE_X")); // exact, ikke prefix
        assert!(!nested_scrub_matches("CLAUDE_MODEL")); // uden for begge moenstre
        assert!(!nested_scrub_matches("CLAUDE"));
    }

    #[test]
    fn claude_transcript_root_respects_claude_config_dir() {
        // T8 (B4): CLAUDE_CONFIG_DIR-logikken flyttede hertil fra
        // transcripts.rs' gamle `claude_projects_dir()` — CC-brugere med
        // variablen sat maa IKKE tavst miste deres transcripts.
        let _guard = env_lock();
        std::env::set_var("CLAUDE_CONFIG_DIR", r"C:\fake-claude-config-dir-t8");
        let root = super::claude_transcript_root();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        assert_eq!(
            root,
            std::path::PathBuf::from(r"C:\fake-claude-config-dir-t8\projects")
        );
    }

    #[test]
    fn codex_profile_exists_with_spike_locked_data() {
        let p = super::profile("codex").expect("codex-profilen findes (spec rev 1.2)");
        assert_eq!(p.spawn_command, &["codex"]);
        assert_eq!(p.resume_command, &["codex", "resume", "--last"]);
        assert!(p.env_deny_prefixes.contains(&"OPENAI_"));
        let spec = p.readiness.expect("codex har readiness-spec");
        assert!(!spec.require_alt_screen, "spike: codex-TUI koerer inline");
        assert!(
            spec.require_column_three,
            "spike: input-cursor i kolonne 3 som CC"
        );
        assert!(spec.live_prompts.contains(&&b"\x1b[1m\r\n\xE2\x80\xBA"[..]));
    }

    #[test]
    fn claude_readiness_matches_current_fsm_constants() {
        let p = super::profile("claude").unwrap();
        let spec = p.readiness.expect("claude har readiness-spec");
        assert!(spec.require_alt_screen);
        assert!(spec.require_column_three);
        assert!(spec.live_prompts.contains(&&b"\xE2\x9D\xAF\xC2\xA0"[..]));
        assert!(spec.live_prompts.contains(&&b">\xC2\xA0"[..]));
    }

    #[test]
    fn readiness_for_command_binds_to_spawn_program_stem() {
        // master-eksklusionen (K1): claude-profil + fremmed executable => ingen FSM
        assert!(super::readiness_for_command("claude", "claude").is_some());
        assert!(super::readiness_for_command("claude", r"C:\x\CLAUDE.EXE").is_some());
        assert!(super::readiness_for_command("claude", "uv").is_none());
        assert!(super::readiness_for_command("codex", "codex").is_some());
        assert!(super::readiness_for_command("codex", "claude").is_none());
        assert!(super::readiness_for_command("ukendt", "claude").is_none());
    }

    /// Test-profil uden PATH-hit og uden fallback (resolve_spawn_program-fejlvejen).
    static TEST_MISSING: AgentProfile = AgentProfile {
        id: "test-missing",
        spawn_command: &["findes-ikke-xyz"],
        resume_command: &["findes-ikke-xyz"],
        env_deny_prefixes: &[],
        env_deny_exact: &[],
        submit_gap_ms: 0,
        transcript_root: || std::path::PathBuf::new(),
        readiness: None,
        mcp: super::McpInjection::ClaudeFlags,
        exe_fallback: None,
        attention_patterns: &[],
    };

    /// Test-profil MED et PATH-hit — men et hit testen selv laver.
    static TEST_PRESENT: AgentProfile = AgentProfile {
        id: "test-present",
        spawn_command: &["findes-paa-path-xyz"],
        resume_command: &["findes-paa-path-xyz"],
        env_deny_prefixes: &[],
        env_deny_exact: &[],
        submit_gap_ms: 0,
        transcript_root: || std::path::PathBuf::new(),
        readiness: None,
        mcp: super::McpInjection::ClaudeFlags,
        exe_fallback: None,
        attention_patterns: &[],
    };

    /// PATH-hit-vejen returnerer inputtet BIT-IDENTISK; fravaer giver en
    /// beskrivende fejl.
    ///
    /// Testen byggede indtil OSS-fase 2 sit PATH-hit paa at `claude.exe` laa
    /// paa maskinens PATH — kommentaren sagde det ligefrem ("findes i
    /// testmiljoeet"). Det var en EJER-MASKINE-ANTAGELSE forkleddt som en
    /// test: den var groen hos den ene der havde Claude Code installeret, og
    /// roed for enhver anden. CI fandt den paa foerste koersel
    /// (`profiles.rs:353`, 2026-08-02), og den ramte fase 2's barre direkte —
    /// *klon, `cargo test`, groent*.
    ///
    /// Nu laver testen sit eget hit: en temp-mappe med en tom fil ved navn
    /// `<program>.exe` og PATH peget derhen. `resolve_spawn_program` spoerger
    /// kun `is_file()`, saa filen behoever ikke vaere en rigtig binaer — og
    /// programnavnet er opdigtet, saa ingen installeret binaer kan forstyrre
    /// maalingen i nogen retning. Ingen anden maskine end denne proces er
    /// involveret.
    #[test]
    fn resolve_spawn_program_returns_input_on_path_hit_and_errors_when_absent() {
        let _g = env_lock();
        let dir = tempfile::tempdir().expect("temp path-mappe");
        std::fs::write(dir.path().join("findes-paa-path-xyz.exe"), b"").expect("laeg exe paa PATH");

        let foer = std::env::var_os("PATH");
        std::env::set_var("PATH", dir.path());
        let hit = super::resolve_spawn_program(&TEST_PRESENT);
        let mangler = super::resolve_spawn_program(&TEST_MISSING);
        match foer {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }

        assert_eq!(
            hit.expect("PATH-hit"),
            "findes-paa-path-xyz",
            "PATH-hit skal give inputtet uaendret — CC-vejen er bit-identisk"
        );
        let err = mangler.expect_err("uden PATH-hit og uden fallback");
        assert!(
            err.contains("findes-ikke-xyz.exe"),
            "fejlen ER brugerteksten og skal navngive den manglende binaer: {err}"
        );
    }
}
