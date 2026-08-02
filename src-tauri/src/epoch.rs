//! Per-kort epoch-gate (spec §6, regel 1+4) — serialiseringspunktet mellem
//! menneske- og persona-input ved pty-grænsen (spec §5, auto-pause).
//!
//! Regel 1: pause/takeover (auto-pause ved menneske-tast) bumper epoch.
//! Regel 4: appen håndhæver epoch MEKANISK — en stale persona-write afvises
//! atomart, før bytes rører pty'en. Menneske- og persona-input blandes aldrig.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Persona,
    Human,
}

pub struct EpochGate {
    epoch: AtomicU64,
    owner: Mutex<Owner>,
}

impl EpochGate {
    /// Nyt kort: epoch 0, personaen har roret (ingen pause endnu).
    pub fn new() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            owner: Mutex::new(Owner::Persona),
        }
    }

    /// Auto-pause/takeover (§6 regel 1): Human tager roret, epoch bumpes.
    /// Returnerer den nye epoch.
    pub fn bump_to_human(&self) -> u64 {
        let mut owner = self.owner.lock().expect("owner lock poisoned");
        *owner = Owner::Human;
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Genoptag: personaen får roret igen; epoch bumpes (nyt kontrol-vindue).
    pub fn resume_to_persona(&self) -> u64 {
        let mut owner = self.owner.lock().expect("owner lock poisoned");
        *owner = Owner::Persona;
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Den mekaniske gate (§6 regel 4): matcher den præsenterede epoch?
    pub fn check(&self, presented: u64) -> bool {
        presented == self.epoch.load(Ordering::SeqCst)
    }

    pub fn current(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    pub fn owner(&self) -> Owner {
        *self.owner.lock().expect("owner lock poisoned")
    }
}

impl Default for EpochGate {
    fn default() -> Self {
        Self::new()
    }
}

/// write_pty-beslutningen (spec §5) som REN, testbar funktion.
///
///   source=="human":    ejer Human allerede? → PassThrough. Ellers → atomisk
///                       takeover (bump) og PauseThenPass — signalfil, event og
///                       bytes-igennem er KALDERENS sideeffekter (main.rs).
///   source=="terminal": ALTID PassThrough (fix F1) — xterm.js' protokol-
///                       auto-svar (CPR/DA/fokus/kitty) er ikke intention:
///                       ingen pause-semantik, intet bump, ingen epoch-check.
///   source=="persona" (og alt ukendt — strammeste gren): PassThrough KUN hvis
///                       personaen har roret OG epoch matcher; ellers RejectStale.
#[derive(Debug, PartialEq, Eq)]
pub enum WriteDecision {
    PassThrough,
    PauseThenPass { new_epoch: u64 },
    RejectStale,
}

pub fn gate_write(gate: &EpochGate, source: &str, presented_epoch: u64) -> WriteDecision {
    if source == "human" {
        // Én lås hen over check+bump: to samtidige menneske-taster giver
        // præcis ét bump og én PauseThenPass.
        let mut owner = gate.owner.lock().expect("owner lock poisoned");
        if *owner == Owner::Human {
            WriteDecision::PassThrough
        } else {
            *owner = Owner::Human;
            let new_epoch = gate.epoch.fetch_add(1, Ordering::SeqCst) + 1;
            WriteDecision::PauseThenPass { new_epoch }
        }
    } else if source == "terminal" {
        // Fix F1: terminal-protokol-auto-svar fra xterm (CPR ved senere
        // ESC[6n-queries, DA, fokus-events CSI I/O ved mode 1004 — CC
        // enabler den, spike-FUND 6 — og kitty-svar) SKAL altid leveres,
        // ellers hænger childens query. Grenen står BEVIDST FØR
        // persona-fallbacken: den behandler ukendte sources som strammeste
        // gren og ville ellers epoch-afvise svarene, mens mennesket har
        // roret. Ingen pause, intet bump, ingen owner-ændring.
        WriteDecision::PassThrough
    } else {
        let owner = gate.owner.lock().expect("owner lock poisoned");
        if *owner == Owner::Persona && presented_epoch == gate.epoch.load(Ordering::SeqCst) {
            WriteDecision::PassThrough
        } else {
            WriteDecision::RejectStale
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_gate_starts_at_epoch_zero_owned_by_persona() {
        let g = EpochGate::new();
        assert_eq!(g.current(), 0);
        assert_eq!(g.owner(), Owner::Persona);
        assert!(g.check(0));
        assert!(!g.check(1));
    }

    #[test]
    fn bump_to_human_increments_epoch_and_takes_ownership() {
        // Spec §6 regel 1: pause/takeover bumper control_epoch.
        let g = EpochGate::new();
        let e = g.bump_to_human();
        assert_eq!(e, 1);
        assert_eq!(g.current(), 1);
        assert_eq!(g.owner(), Owner::Human);
        assert!(!g.check(0), "old epoch must be stale after takeover");
    }

    #[test]
    fn resume_to_persona_increments_again_and_returns_ownership() {
        let g = EpochGate::new();
        g.bump_to_human();
        let e = g.resume_to_persona();
        assert_eq!(e, 2);
        assert_eq!(g.owner(), Owner::Persona);
        assert!(g.check(2));
        assert!(!g.check(1));
    }

    #[test]
    fn gate_write_first_human_key_pauses_then_passes() {
        // Spec §5: første menneske-tast → auto-pause (bump) FØR bytes; bytes
        // skal ALTID igennem. Anden tast: ejeren er allerede Human → ren pass.
        let g = EpochGate::new();
        assert_eq!(
            gate_write(&g, "human", 0),
            WriteDecision::PauseThenPass { new_epoch: 1 }
        );
        assert_eq!(gate_write(&g, "human", 0), WriteDecision::PassThrough);
        assert_eq!(g.current(), 1, "no double bump on repeated human input");
    }

    #[test]
    fn gate_write_persona_with_current_epoch_passes() {
        let g = EpochGate::new();
        assert_eq!(gate_write(&g, "persona", 0), WriteDecision::PassThrough);
    }

    #[test]
    fn gate_write_persona_with_stale_epoch_rejected() {
        // Spec §6 regel 4: den mekaniske gate afviser stale writes.
        let g = EpochGate::new();
        g.bump_to_human();
        g.resume_to_persona(); // epoch = 2, owner = Persona
        assert_eq!(gate_write(&g, "persona", 1), WriteDecision::RejectStale);
        assert_eq!(gate_write(&g, "persona", 0), WriteDecision::RejectStale);
    }

    #[test]
    fn takeover_race_inflight_persona_write_rejected_mechanically() {
        // Racen fra spec §5/§6 regel 4: en persona-write afsendt FØR auto-pausen
        // (in-flight) præsenterer den gamle epoch og skal afvises atomart.
        let g = EpochGate::new();
        let inflight_epoch = g.current(); // 0 — læst før menneske-tasten
        let _ = gate_write(&g, "human", 0); // auto-pause: synkront bump
        assert_eq!(
            gate_write(&g, "persona", inflight_epoch),
            WriteDecision::RejectStale
        );
    }

    #[test]
    fn gate_write_persona_never_passes_while_human_owns() {
        // Spec §5: menneske- og persona-input kan ALDRIG blandes — selv den
        // nye epoch giver ikke persona-adgang, så længe mennesket har roret.
        let g = EpochGate::new();
        let e = g.bump_to_human();
        assert_eq!(gate_write(&g, "persona", e), WriteDecision::RejectStale);
    }

    #[test]
    fn unknown_source_treated_as_persona_strictest_branch() {
        let g = EpochGate::new();
        g.bump_to_human();
        assert_eq!(
            gate_write(&g, "wat", g.current()),
            WriteDecision::RejectStale
        );
    }

    #[test]
    fn terminal_source_always_passes_regardless_of_owner_and_epoch() {
        // Fix F1: xterm.js' protokol-auto-svar (CPR/DA/fokus/kitty) sendes
        // med source="terminal" og skal ALTID leveres — også mens mennesket
        // har roret og med vilkårlig epoch. Uden en eksplicit gren ville
        // persona-fallbacken (strammeste gren) epoch-afvise svarene, og
        // childen ville aldrig få sit CPR. Ingen pause-semantik, intet bump.
        let g = EpochGate::new();
        assert_eq!(gate_write(&g, "terminal", 0), WriteDecision::PassThrough);
        g.bump_to_human();
        assert_eq!(gate_write(&g, "terminal", 0), WriteDecision::PassThrough);
        assert_eq!(gate_write(&g, "terminal", 99), WriteDecision::PassThrough);
        assert_eq!(g.current(), 1, "terminal writes must not bump the epoch");
        assert_eq!(
            g.owner(),
            Owner::Human,
            "terminal writes must not flip owner"
        );
    }
}
