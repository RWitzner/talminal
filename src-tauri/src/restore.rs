//! Restore-on-launch: `claude --continue` + et-resumbart-kort-pr.-cwd-reglen
//! (Task 11). REN plan-logik over WorkspaceCard-slices — ingen registry-/fs-
//! mutation her; main.rs wirer planen ind i startup-stien.
//!
//! Regler (laast beslutning 9 + plan Task 11, bindende; T8/B1 udvider
//! gruppenoeglen med profilen):
//! - Kort med `last_active_at == None` -> `Fresh` UANSET gruppering (aldrig
//!   `--continue` i en cwd uden session — CC ville fejle ind i exit-overlayet).
//! - Oevrige grupperes paa (KANONIKALISERET cwd, profil); pr. gruppe faar
//!   kortet med nyeste `last_active_at` -> `Resume`, resten -> `FreshSharedCwd`.
//!   Et codex- og et claude-kort i SAMME cwd konkurrerer dermed IKKE om
//!   samme resume-slot (T8/B1 — hver profil har sin egen `--continue`/
//!   `resume --last`-adfaerd og transcript-rod).
//! - Spawn-adfaerd (Blocker-fix, spec §4.1-konform): KUN Resume-kort
//!   autospawnes ved launch (med profilens resume_command). FreshSharedCwd/
//!   Fresh spawner ALDRIG automatisk — de venter paa brugerklik (og undgaar
//!   dermed racen hvor en frisk autospawnet CC-session i samme cwd kunne
//!   blive den "nyeste" og stjaele Resume-kortets `--continue`).

use std::collections::HashMap;
use std::path::Path;

use crate::workspace::WorkspaceCard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreAction {
    Resume,
    FreshSharedCwd,
    Fresh,
}

impl RestoreAction {
    /// Wire-formen i `CardInfo.restore_action` (frontendens badge, Task 9/10).
    pub fn as_str(self) -> &'static str {
        match self {
            RestoreAction::Resume => "resume",
            RestoreAction::FreshSharedCwd => "fresh_shared_cwd",
            RestoreAction::Fresh => "fresh",
        }
    }
}

/// Cwd-kanonikalisering til gruppenoeglen — JOURNALFOERT VALG (Windows-
/// semantik, plan Task 11 testcase (d)):
/// - Eksisterende stier: `std::fs::canonicalize` — oploeser case-varianter,
///   separator-varianter, relative segmenter, symlinks og 8.3-korte navne til
///   EN form (`\\?\`-praefikset er harmloest som noegle; dunce er ikke i
///   dependency-saettet, og noeglen vises aldrig for brugeren).
/// - Doede stier (canonicalize fejler): tekstuel fallback — `/` -> `\`,
///   trailing-separator-trim og ASCII-lowercase (Windows-fs er
///   case-insensitivt; NTFS' opt-in case-sensitive-dirs er en accepteret
///   blind vinkel i MVP).
///
/// Resultatet lowercases i BEGGE grene saa noeglen er case-stabil.
fn canonical_key(cwd: &str) -> String {
    match std::fs::canonicalize(Path::new(cwd)) {
        Ok(p) => p.to_string_lossy().to_ascii_lowercase(),
        Err(_) => {
            let mut s = cwd.replace('/', "\\");
            while s.len() > 3 && s.ends_with('\\') {
                s.pop();
            }
            s.to_ascii_lowercase()
        }
    }
}

/// Planen: et (name, action)-par pr. kort, i input-orden.
///
/// "Nyeste" sammenlignes leksikografisk paa `last_active_at` — formatet er
/// fast `YYYY-MM-DDTHH:MM:SS.mmmZ` (workspace.rs' now_iso_z), saa
/// leksikografisk == kronologisk. Ved identisk timestamp vinder hoejeste
/// kortnummer (numre er unikke => fuldt deterministisk).
pub fn restore_plan(cards: &[WorkspaceCard]) -> Vec<(String, RestoreAction)> {
    // Noegler beregnes EN gang pr. kort (canonicalize rammer filsystemet).
    // None-kort faar ingen noegle: de deltager ikke i grupperingen.
    // T8/B1: noeglen er (kanonikaliseret cwd, profil) — ikke cwd alene, saa
    // en codex- og en claude-session i samme cwd faar hver sit resume-slot.
    let keys: Vec<Option<(String, String)>> = cards
        .iter()
        .map(|c| {
            c.last_active_at
                .as_ref()
                .map(|_| (canonical_key(&c.cwd), c.profile.clone()))
        })
        .collect();
    // Vinder-indeks pr. gruppe: max paa (last_active_at, number).
    let mut winner: HashMap<(&str, &str), usize> = HashMap::new();
    for (i, card) in cards.iter().enumerate() {
        let Some(key) = keys[i]
            .as_ref()
            .map(|(cwd, profile)| (cwd.as_str(), profile.as_str()))
        else {
            continue;
        };
        let candidate = (card.last_active_at.as_deref().unwrap_or(""), card.number);
        match winner.get(&key) {
            Some(&w)
                if (
                    cards[w].last_active_at.as_deref().unwrap_or(""),
                    cards[w].number,
                ) >= candidate => {}
            _ => {
                winner.insert(key, i);
            }
        }
    }
    cards
        .iter()
        .enumerate()
        .map(|(i, card)| {
            let key = keys[i]
                .as_ref()
                .map(|(cwd, profile)| (cwd.as_str(), profile.as_str()));
            let action = match key {
                // Laast beslutning 9: ingen session -> Fresh, uanset gruppering.
                None => RestoreAction::Fresh,
                Some(key) if winner.get(&key) == Some(&i) => RestoreAction::Resume,
                Some(_) => RestoreAction::FreshSharedCwd,
            };
            (card.name.clone(), action)
        })
        .collect()
}
