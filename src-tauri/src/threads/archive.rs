//! Append-only JSONL pr. traad under `talminal_base()/threads/<id>.jsonl`.

use std::io::Write;
use std::path::PathBuf;

use super::{Message, TerminalReason};

pub fn threads_dir() -> PathBuf {
    crate::cards::talminal_base().join("threads")
}

/// Et traad-id er `t` efterfulgt af mindst ét ciffer og intet andet — formen
/// `next_thread_id()` udsteder. Alt andet er enten en fremmed fil i mappen
/// eller en streng fra et sted den ikke burde komme fra.
pub(crate) fn is_thread_id(id: &str) -> bool {
    matches!(id.strip_prefix('t'), Some(digits)
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Talvaerdien i et traad-id. `None` for alt der ikke er et id — og for
/// cifferstrenge saa lange at de ikke er et tal vi nogensinde har udstedt.
pub(crate) fn thread_number(id: &str) -> Option<u64> {
    if !is_thread_id(id) {
        return None;
    }
    id[1..].parse::<u64>().ok()
}

/// `None` for et id der ikke er et traad-id. Valideringen ligger HER og ikke
/// kun hos kalderne, fordi id'et joines paa `threads_dir()`: naaede strengen
/// `../../x` hertil ad en hvilken som helst vej, ville skrivningen forlade
/// mappen. Et ugyldigt id giver ingen sti frem for en tavst omskrevet — en
/// omskrivning ville skjule at nogen naaede hertil med noget der ikke er et id.
pub fn thread_path(id: &str) -> Option<PathBuf> {
    is_thread_id(id).then(|| threads_dir().join(format!("{id}.jsonl")))
}

pub(crate) fn reason_slug(reason: TerminalReason) -> &'static str {
    match reason {
        TerminalReason::Answer => "answer",
        TerminalReason::IdleTimeout { .. } => "idle_timeout",
        TerminalReason::AbsoluteTimeout { .. } => "absolute_timeout",
        TerminalReason::BackstopCleanup => "backstop_cleanup",
        TerminalReason::DeliveryFailed => "delivery_failed",
        TerminalReason::ParticipantLost => "participant_lost",
        TerminalReason::OwnerStopped => "owner_stopped",
        TerminalReason::RestartAbort => "restart_abort",
        TerminalReason::HopLimit => "hop_limit",
    }
}

pub(crate) fn line_for(msg: &Message, reason: Option<TerminalReason>) -> String {
    let mut v = serde_json::json!({
        "seq": msg.seq,
        "thread": msg.thread,
        "from_card": msg.from_card,
        "from_kind": msg.from_kind.as_str(),
        "intent": msg.intent.as_str(),
        "hop": msg.hop,
        "text": msg.text,
        "ts_ms": msg.ts_ms,
    });
    if let Some(r) = reason {
        v["terminal_reason"] = serde_json::Value::String(reason_slug(r).to_string());
        if let TerminalReason::IdleTimeout { peer_active }
        | TerminalReason::AbsoluteTimeout { peer_active } = r
        {
            v["peer_active"] = serde_json::Value::Bool(peer_active);
        }
    }
    v.to_string()
}

pub fn append_lines(thread: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    if let Err(e) = try_append(thread, lines) {
        eprintln!("threads: archive append failed for {thread}: {e}");
    }
}

fn try_append(thread: &str, lines: &[String]) -> Result<(), String> {
    let path = thread_path(thread).ok_or_else(|| format!("ikke et traad-id: {thread:?}"))?;
    std::fs::create_dir_all(threads_dir()).map_err(|e| e.to_string())?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    for line in lines {
        writeln!(f, "{line}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Terminaliserer arkiverede delegeringer der stadig stod `awaiting`, da
/// processen forsvandt. Arkivet genoplives aldrig ved opstart.
///
/// FILNAVNET ER SANDHEDEN om hvilken traad en fil hoerer til. Passet laeste
/// tidligere id'et ud af postens `thread`-felt og brugte det baade som
/// Message::thread og som append-maal, saa en `t6.jsonl` hvis poster paastod
/// `"thread":"t9"` fik sin restart_abort-linje skrevet i `t9.jsonl` — den
/// terminaliserede altsaa en FREMMED samtale. Indholdet er data fra disken;
/// navnet er det eneste vi selv har udstedt.
pub fn terminalize_awaiting_on_startup() -> Result<usize, String> {
    let dir = threads_dir();
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut fixed = 0usize;
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let id = match path.file_stem().and_then(|stem| stem.to_str()) {
            Some(stem) if is_thread_id(stem) => stem.to_string(),
            _ => {
                eprintln!(
                    "threads: springer arkivfil over, navnet er ikke et traad-id: {}",
                    path.display()
                );
                continue;
            }
        };
        let body = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let mut open_delegation = false;
        let mut last_seq = 0u64;
        for line in body.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            // En post der paastaar en anden traad hoerer ikke til i denne fil og
            // faar ikke lov at bestemme dens tilstand.
            let claimed = value["thread"].as_str().unwrap_or_default();
            if claimed != id {
                eprintln!(
                    "threads: {} indeholder en post der paastaar traad {claimed:?} - ignoreret",
                    path.display()
                );
                continue;
            }
            last_seq = last_seq.max(value["seq"].as_u64().unwrap_or(0));
            match value["intent"].as_str() {
                Some("delegation") => open_delegation = true,
                Some("answer") => open_delegation = false,
                _ => {}
            }
            if value.get("terminal_reason").is_some() {
                open_delegation = false;
            }
        }
        if open_delegation {
            let message = super::Message {
                seq: last_seq + 1,
                thread: id.clone(),
                from_card: super::SYSTEM.to_string(),
                from_kind: super::FromKind::System,
                intent: super::Intent::Status,
                hop: 0,
                text: super::terminal_text(super::TerminalReason::RestartAbort, "modparten"),
                ts_ms: super::now_ms(),
            };
            append_lines(
                &id,
                &[line_for(
                    &message,
                    Some(super::TerminalReason::RestartAbort),
                )],
            );
            fixed += 1;
        }
    }
    Ok(fixed)
}
