//! fs-poll af presence-filer (controller.json + session-*.json) -> "presence"-Tauri-event.
//!
//! Selvbaerende rendering (spec §5, §2 inv. 7): appen laeser controllerens
//! heartbeat-/statusfiler direkte fra disken og maler kanterne selv — graa
//! ("kan ikke garanteres") kraever ingen levende controller. Controlleren
//! skriver filerne atomisk (tmp + os.replace), saa en laesning ser altid en
//! hel fil; alligevel er al parsing defensiv: malformede filer vaelter
//! ALDRIG pollen, de udelades blot af payloadet.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Emitter;

/// Spejler controllerens session-statusfil `session-<key>.json`
/// (skrevet af controllerens presence.py, Task 3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub schema_version: u32,
    pub persona_session: Option<String>,
    pub claude_session_id: String,
    pub execution_state: String,
    pub control_owner: String,
    pub health: String,
    pub attention_state: String,
    pub updated_at: String,
}

/// Spejler `controller.json` — kun `ts` bruges videre.
#[derive(Debug, Clone, Deserialize)]
struct ControllerHeartbeat {
    #[allow(dead_code)]
    schema_version: u32,
    ts: String,
}

/// Payload for det kanoniske "presence"-event:
/// { controller_ts: String|null, sessions: { [key]: SessionStatus } }.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PresencePayload {
    pub controller_ts: Option<String>,
    pub sessions: HashMap<String, SessionStatus>,
}

/// Basedir-reglen deles med controllerens config.py::default_paths:
/// TALMINAL_HOME-env ellers %LOCALAPPDATA%\Talminal — plus "presence".
/// En TOM TALMINAL_HOME behandles som fravaerende, praecis som Pythons
/// `os.environ.get(...) or ...` (ellers ville "" give den relative sti "presence").
pub fn presence_dir() -> PathBuf {
    // Genbrug Task 7's kanoniske base-dir-oploesning (TALMINAL_HOME-override,
    // tom streng = fravaerende, ellers %LOCALAPPDATA%\Talminal) — ingen tredje
    // kopi af reglen (samme princip som Task 8's main.rs).
    crate::cards::talminal_base().join("presence")
}

/// Laeser presence-dir'et defensivt. Manglende dir, manglende filer og
/// malformet JSON giver blot None/udeladelse — aldrig panic eller fejl.
pub fn read_presence(dir: &Path) -> PresencePayload {
    let controller_ts = std::fs::read_to_string(dir.join("controller.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<ControllerHeartbeat>(&s).ok())
        .map(|hb| hb.ts);

    let mut sessions = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            let key = match name
                .strip_prefix("session-")
                .and_then(|rest| rest.strip_suffix(".json"))
            {
                Some(k) if !k.is_empty() => k.to_string(),
                _ => continue, // controller.json, *.tmp, alt andet
            };
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(status) = serde_json::from_str::<SessionStatus>(&text) else {
                continue; // malformet fil udelades — vaelter aldrig pollen
            };
            sessions.insert(key, status);
        }
    }
    PresencePayload {
        controller_ts,
        sessions,
    }
}

/// Starter poll-traaden: hvert 500 ms laeses dir'et og "presence"-eventet
/// emittes UBETINGET (frontenden genberegner stale-reglen mod Date.now()
/// ved hvert event, saa 10 s-graenserne opdager sig selv uden ekstra timer).
pub fn spawn_presence_poller(app: tauri::AppHandle, dir: PathBuf) {
    std::thread::spawn(move || loop {
        let payload = read_presence(&dir);
        let _ = app.emit("presence", &payload);
        std::thread::sleep(Duration::from_millis(500));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const CONTROLLER: &str = r#"{"schema_version":1,"ts":"2026-07-17T12:00:00.000Z","run_id":"11111111-1111-7111-8111-111111111111","pid":4242}"#;

    fn session_json(persona: &str, execution_state: &str) -> String {
        format!(
            r#"{{"schema_version":1,"persona_session":"{persona}","claude_session_id":"00000000-0000-4000-8000-000000000001","execution_state":"{execution_state}","control_owner":"persona","health":"healthy","attention_state":"none","updated_at":"2026-07-17T12:00:00.000Z"}}"#
        )
    }

    #[test]
    fn missing_dir_gives_empty_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let p = read_presence(&missing);
        assert_eq!(p.controller_ts, None);
        assert!(p.sessions.is_empty());
    }

    #[test]
    fn controller_heartbeat_ts_is_read() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("controller.json"), CONTROLLER).unwrap();
        let p = read_presence(tmp.path());
        assert_eq!(p.controller_ts.as_deref(), Some("2026-07-17T12:00:00.000Z"));
    }

    #[test]
    fn malformed_controller_json_gives_none_but_sessions_still_read() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("controller.json"), "{not json").unwrap();
        fs::write(
            tmp.path().join("session-a.json"),
            session_json("a", "generating"),
        )
        .unwrap();
        let p = read_presence(tmp.path());
        assert_eq!(p.controller_ts, None);
        assert_eq!(p.sessions.len(), 1);
        assert!(p.sessions.contains_key("a"));
    }

    #[test]
    fn session_files_keyed_by_filename_key() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("controller.json"), CONTROLLER).unwrap();
        fs::write(
            tmp.path().join("session-a.json"),
            session_json("a", "generating"),
        )
        .unwrap();
        fs::write(
            tmp.path().join("session-b.json"),
            session_json("b", "waiting_permission"),
        )
        .unwrap();
        let p = read_presence(tmp.path());
        assert_eq!(p.controller_ts.as_deref(), Some("2026-07-17T12:00:00.000Z"));
        assert_eq!(p.sessions.len(), 2);
        assert_eq!(p.sessions["a"].execution_state, "generating");
        assert_eq!(p.sessions["b"].execution_state, "waiting_permission");
        assert_eq!(p.sessions["a"].persona_session.as_deref(), Some("a"));
    }

    #[test]
    fn malformed_session_file_is_skipped_valid_kept() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("session-broken.json"), "{truncated").unwrap();
        fs::write(
            tmp.path().join("session-a.json"),
            session_json("a", "idle_at_prompt"),
        )
        .unwrap();
        let p = read_presence(tmp.path());
        assert_eq!(p.sessions.len(), 1);
        assert!(p.sessions.contains_key("a"));
    }

    #[test]
    fn unrelated_and_tmp_files_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("session-a.json"),
            session_json("a", "generating"),
        )
        .unwrap();
        fs::write(
            tmp.path().join("session-a.json.tmp"),
            session_json("a", "generating"),
        )
        .unwrap();
        fs::write(tmp.path().join("notes.txt"), "ignore me").unwrap();
        let p = read_presence(tmp.path());
        assert_eq!(p.sessions.len(), 1);
        assert!(p.sessions.contains_key("a"));
    }
}
