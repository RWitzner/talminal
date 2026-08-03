//! Usage-HUD-snapshottet: læser den GLOBALE <global_base>\hud\usage.json —
//! skrevet af statusline-tap'en (canvas/statusline-tap/tap.mjs). usage.json er
//! konto-niveau-data, og canvas'en sætter TALMINAL_HOME per-projekt ved
//! opstart, så basen er project::global_base() (TALMINAL_GLOBAL_HOME-override,
//! ellers %LOCALAPPDATA%\Talminal) — IKKE cards::talminal_base(). Tap'en
//! opløser præcis samme base. Kontrakt v1 (FROSSEN): camelCase-felter,
//! procenter 0-100 (klampes defensivt her), resets som ISO-8601-strenge.
//! Friskhed vurderes IKKE her — frontenden dømmer alder mod sit eget ur, så
//! klient og backend aldrig bruger hver sin klokke.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::project;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub version: u32,
    pub written_at: String,
    pub five_hour_percent: f64,
    #[serde(default)]
    pub five_hour_resets_at: Option<String>,
    #[serde(default)]
    pub weekly_percent: Option<f64>,
    #[serde(default)]
    pub weekly_resets_at: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

use crate::context_hud::clamp_percent;

pub fn read_snapshot_at(path: &Path) -> Option<UsageSnapshot> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut snapshot: UsageSnapshot = serde_json::from_str(&text).ok()?;
    if snapshot.version != 1 {
        return None;
    }
    snapshot.five_hour_percent = clamp_percent(snapshot.five_hour_percent)?;
    snapshot.weekly_percent = snapshot.weekly_percent.and_then(clamp_percent);
    Some(snapshot)
}

pub fn snapshot_path() -> PathBuf {
    project::global_base().join("hud").join("usage.json")
}

pub fn read_snapshot() -> Option<UsageSnapshot> {
    read_snapshot_at(&snapshot_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unik temp-mappe pr. test — ingen env-mutation, ingen oprydningskrav
    /// (OS-temp), ingen nye dev-dependencies.
    fn write_temp(name: &str, content: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("usage-hud-test-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("usage.json");
        std::fs::write(&path, content).expect("write usage.json");
        path
    }

    #[test]
    fn parses_full_snapshot() {
        let path = write_temp(
            "full",
            r#"{"version":1,"writtenAt":"2026-07-21T19:00:00.000Z","fiveHourPercent":5.0,"fiveHourResetsAt":"2026-07-21T22:20:00.000Z","weeklyPercent":1.0,"weeklyResetsAt":"2026-07-28T07:00:00.000Z","sessionId":"abc"}"#,
        );
        let snapshot = read_snapshot_at(&path).expect("snapshot");
        assert_eq!(snapshot.five_hour_percent, 5.0);
        assert_eq!(snapshot.weekly_percent, Some(1.0));
        assert_eq!(snapshot.written_at, "2026-07-21T19:00:00.000Z");
        assert_eq!(snapshot.session_id.as_deref(), Some("abc"));
    }

    #[test]
    fn tolerates_missing_optionals_and_clamps() {
        let path = write_temp(
            "optional",
            r#"{"version":1,"writtenAt":"2026-07-21T19:00:00.000Z","fiveHourPercent":130.0}"#,
        );
        let snapshot = read_snapshot_at(&path).expect("snapshot");
        assert_eq!(snapshot.five_hour_percent, 100.0);
        assert_eq!(snapshot.weekly_percent, None);
        assert_eq!(snapshot.five_hour_resets_at, None);
        assert_eq!(snapshot.session_id, None);
    }

    #[test]
    fn rejects_wrong_version_missing_file_and_garbage() {
        let wrong_version = write_temp(
            "version",
            r#"{"version":2,"writtenAt":"x","fiveHourPercent":5.0}"#,
        );
        assert!(read_snapshot_at(&wrong_version).is_none());
        assert!(read_snapshot_at(Path::new("Z:\\findes\\ikke\\usage.json")).is_none());
        let garbage = write_temp("garbage", "ikke json");
        assert!(read_snapshot_at(&garbage).is_none());
    }

    #[test]
    fn snapshot_path_er_hud_usage_json_under_basen() {
        // Base-opløsningen (TALMINAL_GLOBAL_HOME-override vinder, tom
        // ignoreres, LOCALAPPDATA-fallback) er dækket af project.rs' egne
        // global_base-tests under env_lock — env muteres IKKE her (modulets
        // testprincip). Suffixet er det eneste usage_hud selv bidrager med.
        assert!(snapshot_path().ends_with(Path::new("hud").join("usage.json")));
    }

    #[test]
    fn rejects_out_of_range_number() {
        // serde_json afviser typisk selv 1e400 ved parse; clamp-guarden dækker
        // begge veje (parse-fejl ELLER ikke-endeligt tal) — resultatet er None.
        let path = write_temp(
            "range",
            r#"{"version":1,"writtenAt":"x","fiveHourPercent":1e400}"#,
        );
        assert!(read_snapshot_at(&path).is_none());
    }
}
