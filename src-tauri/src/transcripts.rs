//! Transcript-tail-laesning — profil-bevidst rod (Task 8): Claude Code's
//! `~/.claude/projects` og Codex' `~/.codex/sessions` via profilernes egen
//! `transcript_root`-fn (profiles.rs). Ukendt profil -> tom hale, ALDRIG
//! claude-fallback (GPT-review-krav).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::profiles;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptTail {
    pub entries: Vec<TranscriptEntry>,
    pub per_directory: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub role: String,
    pub text: String,
    pub ts: String,
}

/// Claude Code maps a cwd to a project directory by replacing every path
/// separator and the Windows drive colon independently with `-`.
pub fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' => '-',
            other => other,
        })
        .collect()
}

fn newest_jsonl(directory: &Path) -> Result<Option<PathBuf>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "read transcript directory '{}' failed: {error}",
                directory.display()
            ))
        }
    };

    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read transcript entry failed: {error}"))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if newest.as_ref().is_none_or(|(current, current_path)| {
            modified > *current || (modified == *current && path > *current_path)
        }) {
            newest = Some((modified, path));
        }
    }
    Ok(newest.map(|(_, path)| path))
}

fn text_content(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        let text = text.trim();
        return (!text.is_empty()).then(|| text.to_string());
    }
    let blocks = content.as_array()?;
    let text = blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn parse_entry(line: &str) -> Option<TranscriptEntry> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let message = value.get("message")?;
    let role = message.get("role")?.as_str()?;
    if role != "user" && role != "assistant" {
        return None;
    }
    let text = text_content(message.get("content")?)?;
    let ts = value
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(TranscriptEntry {
        role: role.to_string(),
        text,
        ts,
    })
}

pub fn read_transcript_tail_from_projects(
    projects_dir: &Path,
    cwd: &Path,
    max_entries: u32,
) -> Result<TranscriptTail, String> {
    let empty = || TranscriptTail {
        entries: Vec::new(),
        per_directory: true,
    };
    if max_entries == 0 {
        return Ok(empty());
    }

    let project_dir = projects_dir.join(project_slug(cwd));
    let Some(path) = newest_jsonl(&project_dir)? else {
        return Ok(empty());
    };
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("read transcript '{}' failed: {error}", path.display()))?;
    let mut entries = contents.lines().filter_map(parse_entry).collect::<Vec<_>>();
    let keep = max_entries as usize;
    if entries.len() > keep {
        entries.drain(..entries.len() - keep);
    }
    Ok(TranscriptTail {
        entries,
        per_directory: true,
    })
}

/// Slaar transcript-roden op for en profil via profiles-modulet (Task 1).
/// Ukendt profil -> `None` -> tom hale, ALDRIG claude-fallback (GPT-review:
/// en ukendt profil maa ikke kunne laese CC's transcripts).
fn root_for_profile(profile_id: &str) -> Option<PathBuf> {
    profiles::profile(profile_id).map(|prof| (prof.transcript_root)())
}

/// Test-seam: samme profil-routing som `read_transcript_tail_for_profile`,
/// men med injicerede rødder i stedet for profilernes egne env-opslag —
/// laaser B4 uden at afhaenge af det rigtige filsystem/miljoe. Kun de to
/// kendte profiler har en rod her; enhver anden profil-id giver tom hale
/// UANSET hvad de injicerede rødder indeholder.
pub fn read_transcript_tail_for_profile_with_roots(
    profile_id: &str,
    cwd: &str,
    max_entries: u32,
    claude_root: &Path,
    codex_root: &Path,
) -> Result<TranscriptTail, String> {
    let empty = || TranscriptTail {
        entries: Vec::new(),
        per_directory: true,
    };
    let root = match profile_id {
        "claude" => claude_root,
        "codex" => codex_root,
        _ => return Ok(empty()),
    };
    read_transcript_tail_from_projects(root, Path::new(cwd), max_entries)
}

/// Offentlig, profil-bevidst indgang (main.rs' `read_transcript_tail`-
/// kommando): roden udledes af profilens egen `transcript_root`-fn
/// (env-opslag i produktion), og delegerer til test-seamet ovenfor.
pub fn read_transcript_tail_for_profile(
    profile_id: &str,
    cwd: &str,
    max_entries: u32,
) -> Result<TranscriptTail, String> {
    let claude_root = root_for_profile("claude").unwrap_or_default();
    let codex_root = root_for_profile("codex").unwrap_or_default();
    read_transcript_tail_for_profile_with_roots(
        profile_id,
        cwd,
        max_entries,
        &claude_root,
        &codex_root,
    )
}
