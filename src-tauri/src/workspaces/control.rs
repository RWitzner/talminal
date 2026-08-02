//! Lukke-kanalen. En synlig proces kan ikke lukke en skjult gennem dennes lokale
//! teardown — anmodningen skal krydse proces-grænsen.
//!
//! Idempotens: `take_pending` fjerner filen, så tre klik giver én lukning.
//! `quit_all` har prioritet over `close`: når hele Talminal er på vej ned, må
//! et samtidigt rail-klik aldrig nedgradere en allerede skrevet global exit til
//! en almindelig workspace-lukning med handoff.
//!
//! NB: modulet hører til T11, men landede med T8, fordi `set_workspace_hidden`
//! skal kunne lukke et kørende workspace før det fjernes fra listen. T11 bygger
//! funnelen, pollertrådens `take_pending`-hook og testene ovenpå.

use crate::atomic;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseRequest {
    pub request_id: u64,
    pub action: String,
}

pub const CLOSE: &str = "close";
pub const QUIT_ALL: &str = "quit_all";

fn path(global_base: &Path, slug: &str) -> PathBuf {
    global_base.join("projects").join(slug).join("control.json")
}

pub fn request_close(global_base: &Path, slug: &str) -> Result<(), String> {
    write_request(global_base, slug, CLOSE)
}

pub fn request_quit_all(global_base: &Path, slug: &str) -> Result<(), String> {
    write_request(global_base, slug, QUIT_ALL)
}

fn write_request(global_base: &Path, slug: &str, action: &str) -> Result<(), String> {
    let existing = std::fs::read_to_string(path(global_base, slug))
        .ok()
        .and_then(|t| serde_json::from_str::<CloseRequest>(&t).ok());

    // Global exit vinder over et senere, samtidigt rail-klik. Polleren fjerner
    // filen efter ét read, så der er ingen permanent tilstand at rydde her.
    if existing
        .as_ref()
        .is_some_and(|request| request.action == QUIT_ALL && action == CLOSE)
    {
        return Ok(());
    }

    let mut body = serde_json::to_string_pretty(&CloseRequest {
        request_id: existing.map(|request| request.request_id).unwrap_or(0) + 1,
        action: action.into(),
    })
    .map_err(|e| e.to_string())?;
    body.push('\n');
    atomic::write(&path(global_base, slug), body.as_bytes()).map_err(|e| e.to_string())
}

/// Henter og FJERNER en ventende anmodning.
pub fn take_pending(global_base: &Path, slug: &str) -> Option<CloseRequest> {
    let p = path(global_base, slug);
    let request: CloseRequest = serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
    let _ = std::fs::remove_file(&p);
    Some(request)
}
