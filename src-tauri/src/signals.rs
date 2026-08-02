//! Pause-/resume-signalfiler (app → controller), spec §5 auto-pause.
//!
//! Appen skriver `<session>.pause.json` / `<session>.resume.json` ATOMISK
//! (tmp + rename) i `<base>\signals\`. Controlleren (consume_signals, Task 4)
//! CLAIMER filen (atomisk rename til `*.processing.<pid>`) FØR læsning,
//! sætter control_owner + audit-linje, og sletter den claimede sti — appens
//! replace på final-stien kolliderer derfor aldrig med controllerens
//! read/delete (fix F7). `<session>` er kortnavnet (= TALMINAL_SESSION_ID).
//! Format (skelet-kanonisk):
//!   {"schema_version": 1, "epoch": <u64>, "ts": "<ISO Z>", "source": "canvas"}

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum SignalError {
    BadSession(String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for SignalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignalError::BadSession(s) => write!(f, "bad session name: {s}"),
            SignalError::Io(e) => write!(f, "signal io error: {e}"),
            SignalError::Json(e) => write!(f, "signal json error: {e}"),
        }
    }
}

impl std::error::Error for SignalError {}

impl From<std::io::Error> for SignalError {
    fn from(e: std::io::Error) -> Self {
        SignalError::Io(e)
    }
}

impl From<serde_json::Error> for SignalError {
    fn from(e: serde_json::Error) -> Self {
        SignalError::Json(e)
    }
}

/// Samme tidsformat som controlleren/proben: YYYY-MM-DDTHH:MM:SS.mmmZ.
pub fn now_iso_z() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// Kortnavne kommer fra lokal, trusted cards.toml — men de bliver til
/// filnavne, så alt der kan ændre stien afvises (bælte og seler).
fn validate_session(session: &str) -> Result<(), SignalError> {
    let ok = !session.is_empty()
        && session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(SignalError::BadSession(session.to_string()))
    }
}

fn write_signal(
    signals_dir: &Path,
    session: &str,
    kind: &str,
    epoch: u64,
) -> Result<PathBuf, SignalError> {
    validate_session(session)?;
    fs::create_dir_all(signals_dir)?;
    let final_path = signals_dir.join(format!("{session}.{kind}.json"));
    let tmp_path = signals_dir.join(format!("{session}.{kind}.json.tmp"));
    let body = serde_json::json!({
        "schema_version": 1,
        "epoch": epoch,
        "ts": now_iso_z(),
        "source": "canvas",
    });
    let mut f = fs::File::create(&tmp_path)?;
    f.write_all(serde_json::to_string(&body)?.as_bytes())?;
    f.sync_all()?;
    drop(f);
    // std::fs::rename bruger MOVEFILE_REPLACE_EXISTING på Windows — en
    // efterladt signalfil (controller offline) overskrives i stedet for
    // at fejle; nyeste epoch vinder. Replacen kan aldrig ramme en fil,
    // controlleren er midt i at forbruge: den claimer (renamer VÆK fra
    // final-stien) før læsning (fix F7, controller-Task 4).
    fs::rename(&tmp_path, &final_path)?;
    Ok(final_path)
}

pub fn write_pause_signal(
    signals_dir: &Path,
    session: &str,
    epoch: u64,
) -> Result<PathBuf, SignalError> {
    write_signal(signals_dir, session, "pause", epoch)
}

pub fn write_resume_signal(
    signals_dir: &Path,
    session: &str,
    epoch: u64,
) -> Result<PathBuf, SignalError> {
    write_signal(signals_dir, session, "resume", epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_signals_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "talminal-signals-test-{}-{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn pause_signal_has_canonical_schema_and_no_tmp_leftover() {
        let dir = tmp_signals_dir("pause");
        let p = write_pause_signal(&dir, "a", 3).unwrap();
        assert_eq!(p, dir.join("a.pause.json"));
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["epoch"], 3);
        assert_eq!(v["source"], "canvas");
        let ts = v["ts"].as_str().unwrap();
        // Controller-tidsformatet: YYYY-MM-DDTHH:MM:SS.mmmZ (24 tegn)
        assert_eq!(ts.len(), 24, "ts must be millisecond ISO Z: {ts}");
        assert_eq!(&ts[10..11], "T");
        assert!(ts.ends_with('Z'));
        assert!(
            !dir.join("a.pause.json.tmp").exists(),
            "tmp file must not be left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resume_signal_overwrites_leftover_file() {
        // Controller offline-scenario: en efterladt signalfil skal
        // overskrives med den nyeste epoch — ikke fejle.
        let dir = tmp_signals_dir("resume");
        write_resume_signal(&dir, "a", 4).unwrap();
        let p = write_resume_signal(&dir, "a", 6).unwrap();
        assert_eq!(p, dir.join("a.resume.json"));
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["epoch"], 6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_names_that_could_change_the_path_are_rejected() {
        let dir = tmp_signals_dir("evil");
        assert!(write_pause_signal(&dir, "..\\evil", 1).is_err());
        assert!(write_pause_signal(&dir, "a/b", 1).is_err());
        assert!(write_pause_signal(&dir, "", 1).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
