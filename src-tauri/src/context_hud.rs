//! Per-kort context-snapshots: læser <global_base>\hud\context\*.json —
//! skrevet af statusline-tap'en (canvas/statusline-tap/tap.mjs) KUN for
//! kort-sessioner (TALMINAL_SESSION_ID arves fra PTY-spawnen). Basen er
//! project::global_base() præcis som usage_hud. Kontrakt context v1 (se
//! docs/superpowers/specs/2026-07-22-card-context-badge-design.md):
//! camelCase, obligatoriske cardName/cwd/usedPercent (frontendens join er
//! cardName+cwd), resten nullable. Defekte/ulæselige filer udelades stille —
//! aldrig en fejl. Friskhed dømmes ikke her (spejler usage_hud-princippet);
//! badgen kræver et kørende kort, så forældede filer er harmløse.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::project;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    pub version: u32,
    pub written_at: String,
    pub card_name: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub cwd: String,
    pub used_percent: f64,
    #[serde(default)]
    pub window_size: Option<f64>,
    #[serde(default)]
    pub model_display_name: Option<String>,
}

fn clamp_percent(value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    Some(value.clamp(0.0, 100.0))
}

fn read_snapshot_file(path: &Path) -> Option<ContextSnapshot> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut snapshot: ContextSnapshot = serde_json::from_str(&text).ok()?;
    if snapshot.version != 1 {
        return None;
    }
    snapshot.used_percent = clamp_percent(snapshot.used_percent)?;
    Some(snapshot)
}

/// Læser alle gyldige context-snapshots i mappen. Manglende mappe = tom
/// liste. Sorteret på cardName for deterministisk output (read_dir-rækkefølge
/// er OS-afhængig).
pub fn read_snapshots_at(dir: &Path) -> Vec<ContextSnapshot> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut snapshots: Vec<ContextSnapshot> = entries
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        })
        .filter_map(|entry| read_snapshot_file(&entry.path()))
        .collect();
    snapshots.sort_by(|a, b| a.card_name.cmp(&b.card_name));
    snapshots
}

pub fn context_dir() -> PathBuf {
    project::global_base().join("hud").join("context")
}

pub fn read_snapshots() -> Vec<ContextSnapshot> {
    read_snapshots_at(&context_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unik temp-mappe pr. test — ingen env-mutation, ingen oprydningskrav
    /// (OS-temp), spejler usage_hud-testernes princip.
    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("context-hud-test-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write(dir: &Path, file: &str, content: &str) {
        std::fs::write(dir.join(file), content).expect("write snapshot file");
    }

    const FULL: &str = r#"{"version":1,"writtenAt":"2026-07-22T09:00:00.000Z","cardName":"kort-3","runId":"run-1","sessionId":"cc-1","cwd":"C:\\proj","usedPercent":18.0,"windowSize":1000000,"modelDisplayName":"Fable 5"}"#;

    #[test]
    fn parses_full_snapshot_and_sorts_by_card_name() {
        let dir = temp_dir("full");
        write(&dir, "kort-9.json", &FULL.replace("kort-3", "kort-9"));
        write(&dir, "kort-3.json", FULL);
        let snapshots = read_snapshots_at(&dir);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].card_name, "kort-3");
        assert_eq!(snapshots[0].used_percent, 18.0);
        assert_eq!(snapshots[0].cwd, "C:\\proj");
        assert_eq!(snapshots[0].model_display_name.as_deref(), Some("Fable 5"));
        assert_eq!(snapshots[1].card_name, "kort-9");
    }

    #[test]
    fn tolerates_missing_optionals_and_clamps() {
        let dir = temp_dir("optional");
        write(
            &dir,
            "kort-1.json",
            r#"{"version":1,"writtenAt":"x","cardName":"kort-1","cwd":"C:\\p","usedPercent":130.0}"#,
        );
        let snapshots = read_snapshots_at(&dir);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].used_percent, 100.0);
        assert_eq!(snapshots[0].run_id, None);
        assert_eq!(snapshots[0].window_size, None);
        assert_eq!(snapshots[0].model_display_name, None);
    }

    #[test]
    fn skips_invalid_files_but_keeps_valid_ones() {
        let dir = temp_dir("invalid");
        write(&dir, "god.json", FULL);
        write(&dir, "skrald.json", "ikke json");
        write(
            &dir,
            "forkert-version.json",
            r#"{"version":2,"writtenAt":"x","cardName":"k","cwd":"C:\\p","usedPercent":5.0}"#,
        );
        write(
            &dir,
            "mangler-cwd.json",
            r#"{"version":1,"writtenAt":"x","cardName":"k","usedPercent":5.0}"#,
        );
        write(&dir, "ikke-json.txt", "ignoreres på extension");
        let snapshots = read_snapshots_at(&dir);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].card_name, "kort-3");
    }

    #[test]
    fn missing_dir_is_empty_list() {
        assert!(read_snapshots_at(Path::new("Z:\\findes\\ikke\\context")).is_empty());
    }

    #[test]
    fn context_dir_er_hud_context_under_basen() {
        // Base-opløsningen er dækket af project.rs' global_base-tests under
        // env_lock — env muteres ikke her (modulets testprincip).
        assert!(context_dir().ends_with(Path::new("hud").join("context")));
    }
}
