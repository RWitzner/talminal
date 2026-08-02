use std::fs;

use serde_json::json;
use talminal_canvas_lib::voice_capture::{append_capture_at, reset_capture_at};
use tempfile::tempdir;

#[test]
fn capture_is_jsonl_and_proves_zero_actions() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("voice-capture.jsonl");
    reset_capture_at(&path).expect("reset capture");

    append_capture_at(
        &path,
        &json!({
            "ts": "2026-07-18T12:00:00Z",
            "transcript": "Genstart kort et",
            "tool": {"name": "restart_card", "arguments": {"card": 1}},
            "resolver": {"ok": true, "card": 1},
            "latency_ms": 321,
            "action_count": 0
        }),
    )
    .expect("append capture");

    let lines = fs::read_to_string(&path).expect("read capture");
    let entries: Vec<serde_json::Value> = lines
        .lines()
        .map(|line| serde_json::from_str(line).expect("jsonl line"))
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["action_count"], 0);
    assert_eq!(entries[0]["tool"]["name"], "restart_card");
}

#[test]
fn capture_rejects_any_nonzero_action_count() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("voice-capture.jsonl");
    let error = append_capture_at(&path, &json!({"transcript": "bad", "action_count": 1}))
        .expect_err("nonzero action count must fail");

    assert!(error.contains("action_count"));
    assert!(!path.exists());
}

#[test]
fn reset_truncates_an_existing_capture() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("voice-capture.jsonl");
    fs::write(&path, "old evidence\n").expect("seed capture");

    reset_capture_at(&path).expect("reset capture");

    assert_eq!(fs::read_to_string(path).expect("read reset capture"), "");
}
