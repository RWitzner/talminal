use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub fn default_capture_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Talminal")
        .join("voice-eval")
        .join("realtime-capture.jsonl")
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "voice capture path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create voice capture directory failed: {error}"))
}

pub fn reset_capture_at(path: &Path) -> Result<(), String> {
    ensure_parent(path)?;
    File::create(path)
        .map(|_| ())
        .map_err(|error| format!("reset voice capture failed: {error}"))
}

pub fn append_capture_at(path: &Path, entry: &Value) -> Result<(), String> {
    if entry.get("action_count").and_then(Value::as_u64) != Some(0) {
        return Err("voice capture requires action_count=0".to_string());
    }
    ensure_parent(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("open voice capture failed: {error}"))?;
    serde_json::to_writer(&mut file, entry)
        .map_err(|error| format!("serialize voice capture failed: {error}"))?;
    file.write_all(b"\n")
        .and_then(|_| file.flush())
        .map_err(|error| format!("append voice capture failed: {error}"))
}

pub fn reset_capture() -> Result<String, String> {
    let path = default_capture_path();
    reset_capture_at(&path)?;
    Ok(path.to_string_lossy().into_owned())
}

pub fn append_capture(entry: &Value) -> Result<String, String> {
    let path = default_capture_path();
    append_capture_at(&path, entry)?;
    Ok(path.to_string_lossy().into_owned())
}
