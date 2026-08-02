//! Rust-side submit-koreografi: vent på Claude-input, skriv prompt-tekst, vent
//! på den verificerede post-text redraw og send først derefter `\r` (ConPTY-
//! empiri, FUND 7).

use std::thread;
use std::time::{Duration, Instant};

use crate::control::{self, WriteOutcome};
use crate::prompt_readiness::PromptWait;
use crate::{cards, profiles, registry, workspace};

const STARTUP_READY_TIMEOUT: Duration = Duration::from_secs(15);
const INPUT_REDRAW_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteSource {
    Human,
    Agent,
}

impl WriteSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            WriteSource::Human => "human",
            WriteSource::Agent => "agent",
        }
    }
}

/// `submit_prompt` gaelder kun terminal-kort. Fejlen skal sige hvilken slags
/// kort der faktisk blev ramt - "browser" om et chat-kort er en vildledning.
fn not_a_terminal(card: &crate::registry::CardRuntime, name: &str) -> String {
    format!("card is a {}: {name}", card.kind_str())
}

/// Sender en hel prompt som menneske-input (epoch 0): først teksten, derefter
/// carriage return når input-cursoren er genaktiveret og profilens minimumsgab
/// er gået. Kort-låsen holdes kun over gate-beslutningen og PtyHost-klonen —
/// aldrig over de blokerende writes eller readiness-vent.
pub fn submit_prompt(name: String, text: String) -> Result<(), String> {
    submit_prompt_as(name, text, WriteSource::Human)
}

pub fn submit_prompt_as(name: String, text: String, source: WriteSource) -> Result<(), String> {
    // Supervision-masteren er et read-only feed og må hverken vente på en
    // Claude-prompt eller modtage bytes gennem denne alternative write-vej.
    cards::guard_write_pty(&name)?;
    let handle = registry::card_handle(&name)?;
    // Første korte lock-scope tager kun identiteten på det aktuelle run og
    // dets startup-gate. Vent aldrig med kort-/registry-låse holdt.
    let (expected_host, readiness) = {
        let card = handle.lock().map_err(|e| e.to_string())?;
        // Browser-kort har ingen PTY at submitte til (plan Task 2-fejlkontrakt).
        let Some(term) = card.terminal() else {
            return Err(not_a_terminal(&card, &name));
        };
        let host = term
            .pty
            .as_ref()
            .cloned()
            .ok_or_else(|| format!("card not running: {name}"))?;
        (host, term.submit_readiness.clone())
    };
    // Exactly one chained submit may own a Claude input widget at a time.
    // Cancellation does not take this mutex and can still wake/kill promptly.
    let _submit_guard = readiness.as_ref().map(|readiness| readiness.lock_submit());

    if let Some(readiness) = &readiness {
        match readiness.wait(STARTUP_READY_TIMEOUT) {
            PromptWait::Ready => {}
            PromptWait::TimedOut => {
                return Err(format!(
                    "card prompt not ready within {}s: {name}",
                    STARTUP_READY_TIMEOUT.as_secs()
                ));
            }
            PromptWait::Cancelled => {
                return Err(format!("card prompt readiness cancelled: {name}"));
            }
        }
    }

    // Gaten kan have ventet, mens kortet blev lukket eller respawnet. Lås
    // derfor igen og bevis, at PTY'en stadig er præcis samme Arc, før epoch-
    // beslutningen og de to writes. Det gamle readiness-signal kan aldrig
    // autorisere input til et nyt run.
    let (host, submit_gap_ms) = {
        let card = handle.lock().map_err(|e| e.to_string())?;
        if !card.active {
            return Err(format!("card is closing: {name}"));
        }
        let Some(term) = card.terminal() else {
            return Err(not_a_terminal(&card, &name));
        };
        let host = term
            .pty
            .as_ref()
            .cloned()
            .ok_or_else(|| format!("card not running: {name}"))?;
        if !std::sync::Arc::ptr_eq(&host, &expected_host) {
            return Err(format!(
                "card process changed while awaiting prompt: {name}"
            ));
        }
        let profile = profiles::profile(&term.profile)
            .ok_or_else(|| format!("unknown agent profile: {}", term.profile))?;

        #[cfg(feature = "supervision")]
        let outcome = control::gate_pty_write(
            &term.gate,
            &crate::cards::talminal_base().join("signals"),
            &name,
            source.as_str(),
            0,
        );
        #[cfg(not(feature = "supervision"))]
        let outcome = control::gate_pty_write(&term.gate, source.as_str(), 0);
        if matches!(outcome, WriteOutcome::RejectStale) {
            return Err("stale_epoch".to_string());
        }

        (host, profile.submit_gap_ms)
    };

    let submit_gap = Duration::from_millis(submit_gap_ms);
    if let Some(readiness) = &readiness {
        if text
            .bytes()
            .all(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            return Err("prompt text is empty".to_string());
        }
        let mut armed_generation = None;
        let write_result = host.write_if_with_output_watermark(text.as_bytes(), |watermark| {
            armed_generation = readiness.arm_text_redraw(watermark, &text);
            armed_generation
        });
        let generation = match write_result {
            Ok(Some(generation)) => generation,
            Ok(None) => {
                return Err(format!("card prompt readiness cancelled: {name}"));
            }
            Err(error) => {
                if let Some(generation) = armed_generation {
                    readiness.disarm_text_redraw(generation);
                }
                readiness.cancel();
                return Err(error.to_string());
            }
        };
        // The profile gap remains a minimum measured from the completed text
        // write; a slow writer may never consume it.
        let gap_started = Instant::now();
        match readiness.wait_for_generation_after(generation, INPUT_REDRAW_TIMEOUT) {
            PromptWait::Ready => {}
            PromptWait::TimedOut => {
                readiness.cancel();
                return Err(format!(
                    "card prompt did not redraw within {}s after text: {name}",
                    INPUT_REDRAW_TIMEOUT.as_secs()
                ));
            }
            PromptWait::Cancelled => {
                return Err(format!("card prompt readiness cancelled: {name}"));
            }
        }
        let remaining_gap = submit_gap.saturating_sub(gap_started.elapsed());
        if !readiness.wait_gap_unless_cancelled(remaining_gap) {
            return Err(format!("card prompt readiness cancelled: {name}"));
        }
    } else {
        host.write(text.as_bytes()).map_err(|e| e.to_string())?;
        thread::sleep(submit_gap);
    }
    // Teardown/respawn kan vinde under gabet. Send aldrig den afsluttende CR
    // til en erstattet eller detached PTY.
    {
        let card = handle.lock().map_err(|e| e.to_string())?;
        if !card.active {
            return Err(format!("card is closing: {name}"));
        }
        let Some(term) = card.terminal() else {
            return Err(not_a_terminal(&card, &name));
        };
        let current = term
            .pty
            .as_ref()
            .ok_or_else(|| format!("card not running: {name}"))?;
        if !std::sync::Arc::ptr_eq(current, &host) {
            return Err(format!("card process changed while submitting: {name}"));
        }
    }
    if let Some(readiness) = &readiness {
        let wrote = match host
            .write_if_with_output_watermark(b"\r", |_| readiness.prepare_submission().then_some(()))
        {
            Ok(wrote) => wrote.is_some(),
            Err(error) => {
                readiness.cancel();
                return Err(error.to_string());
            }
        };
        if !wrote {
            return Err(format!("card prompt readiness cancelled: {name}"));
        }
        if !readiness.mark_submitted(host.output_sequence()) {
            return Err(format!("card prompt readiness cancelled: {name}"));
        }
    } else {
        host.write(b"\r").map_err(|e| e.to_string())?;
    }
    workspace::touch_card_activity(&name);
    Ok(())
}
