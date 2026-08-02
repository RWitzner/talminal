//! Supervisions-facade (Task 1): det ENE punkt hvor main.rs (og integrations-
//! tests) naar pause-/epoch-semantikken. IPC-kontrakten er FROSSEN i begge
//! feature-states (plan Global Constraints):
//!   `write_pty(name, data, epoch, source)` og
//!   `CardState { running, exited, owner, epoch }` beholder signatur/felter.
//!
//! `--features supervision`: re-eksporterer den rigtige `EpochGate` og
//! delegerer til `epoch::gate_write` + `signals::write_*_signal` — adfaerd
//! som i dag, alle eksisterende gate-/pause-tests uaendret groenne.
//!
//! Default (supervision OFF — MVP-pathen): stub-semantik —
//!   - gaten er ALTID pass-through: alle epoch/source-vaerdier accepteres,
//!     bytes skrives altid (aldrig `stale_epoch`)
//!   - owner er altid `"persona"`, epoch altid `0`
//!   - ingen pause-/resume-signalfiler, intet `pause-state`-event
//!   - `resume_card_control` er en no-op der returnerer `Ok(())`
//!
//! Supervision-Rust SLETTES ikke (laast ejer-beslutning): `epoch`/`signals`/
//! `presence` er cfg-gated i lib.rs og vaagner uaendret med featuren.

#[cfg(feature = "supervision")]
mod imp {
    use std::path::Path;

    pub use crate::epoch::EpochGate;
    use crate::epoch::{gate_write, Owner, WriteDecision};
    use crate::signals;

    /// write_pty-beslutningen som main.rs eksekverer:
    ///   `Write`           → bare skriv bytes.
    ///   `WriteAfterPause` → emit "pause-state" og skriv bytes; pause-
    ///                       signalfilen er ALLEREDE skrevet her i facaden
    ///                       (den maa aldrig blokere menneskets tastetryk —
    ///                       fejl logges kun).
    ///   `RejectStale`     → `Err("stale_epoch")`, ingen bytes.
    #[derive(Debug, PartialEq, Eq)]
    pub enum WriteOutcome {
        Write,
        WriteAfterPause { new_epoch: u64 },
        RejectStale,
    }

    pub fn gate_pty_write(
        gate: &EpochGate,
        signals_dir: &Path,
        name: &str,
        source: &str,
        presented_epoch: u64,
    ) -> WriteOutcome {
        match gate_write(gate, source, presented_epoch) {
            WriteDecision::PassThrough => WriteOutcome::Write,
            WriteDecision::RejectStale => WriteOutcome::RejectStale,
            WriteDecision::PauseThenPass { new_epoch } => {
                if let Err(e) = signals::write_pause_signal(signals_dir, name, new_epoch) {
                    eprintln!("pause signal write failed for {name}: {e}");
                }
                WriteOutcome::WriteAfterPause { new_epoch }
            }
        }
    }

    /// get_card_state's owner-felt.
    pub fn card_owner(gate: &EpochGate) -> String {
        match gate.owner() {
            Owner::Human => "human".to_string(),
            Owner::Persona => "persona".to_string(),
        }
    }

    /// get_card_state's epoch-felt.
    pub fn card_epoch(gate: &EpochGate) -> u64 {
        gate.current()
    }

    /// Genoptag (fix F20-raekkefoelgen bevaret): resume-signalfilen skrives
    /// FOER gaten muteres — fejler skrivningen, er gaten uroert. Kaldes under
    /// kort-laasen (main.rs), saa intet andet bump kan skyde sig ind mellem
    /// next-beregningen og resume_to_persona.
    pub fn resume_control(gate: &EpochGate, signals_dir: &Path, name: &str) -> Result<u64, String> {
        let next = gate.current() + 1;
        signals::write_resume_signal(signals_dir, name, next).map_err(|e| e.to_string())?;
        let new_epoch = gate.resume_to_persona();
        debug_assert_eq!(new_epoch, next, "no concurrent bump under the card lock");
        Ok(new_epoch)
    }
}

#[cfg(not(feature = "supervision"))]
mod imp {
    /// ZST-stub med samme konstruktions-API som den rigtige gate — CardRuntime
    /// beholder sit `gate`-felt uaendret i begge feature-states.
    #[derive(Debug, Default)]
    pub struct EpochGate;

    impl EpochGate {
        pub fn new() -> Self {
            EpochGate
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub enum WriteOutcome {
        Write,
        /// Findes kun saa main.rs' match kompilerer i begge states —
        /// konstrueres ALDRIG i default-state.
        WriteAfterPause {
            new_epoch: u64,
        },
        /// Konstrueres ALDRIG i default-state (frossen kontrakt: intet
        /// `stale_epoch`-fejlsvar).
        RejectStale,
    }

    /// Frossen IPC-stub: ALLE epoch/source-vaerdier accepteres, bytes skrives
    /// altid, ingen pause-semantik, ingen signalfiler.
    pub fn gate_pty_write(_gate: &EpochGate, _source: &str, _presented_epoch: u64) -> WriteOutcome {
        WriteOutcome::Write
    }

    /// Frossen kontrakt: owner er altid "persona" i default-state.
    pub fn card_owner(_gate: &EpochGate) -> String {
        "persona".to_string()
    }

    /// Frossen kontrakt: epoch er altid 0 i default-state.
    pub fn card_epoch(_gate: &EpochGate) -> u64 {
        0
    }

    /// Frossen kontrakt: resume_card_control er en no-op — `Ok(())`, ingen
    /// signalfil, intet bump, intet event.
    pub fn resume_control(_gate: &EpochGate) -> Result<(), String> {
        Ok(())
    }
}

pub use imp::*;
