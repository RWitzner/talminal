//! Koalesceret wake-levering uden for MCP-loopet.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::TerminalReason;

pub const IDLE_MS: u64 = 5 * 60 * 1_000;
pub const ABSOLUTE_MS: u64 = 20 * 60 * 1_000;
pub const BACKSTOP_MS: u64 = ABSOLUTE_MS + 60 * 1_000;
pub const MAX_WAKE_ATTEMPTS: u32 = 2;

pub trait Clock: Send + Sync + 'static {
    fn now_ms(&self) -> u64;
}

pub trait Notifier: Send + Sync + 'static {
    fn is_busy(&self, card: &str) -> bool;
    fn write_notice(&self, card: &str, text: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    Nothing,
    Busy,
    Delivered,
    Failed { attempts: u32 },
    DeadLettered,
}

struct Seams {
    clock: Arc<dyn Clock>,
    notifier: Arc<dyn Notifier>,
}

fn seams() -> &'static Mutex<Option<Seams>> {
    static SEAMS: OnceLock<Mutex<Option<Seams>>> = OnceLock::new();
    SEAMS.get_or_init(|| Mutex::new(None))
}

pub fn set_seams(clock: Arc<dyn Clock>, notifier: Arc<dyn Notifier>) {
    *seams().lock().unwrap_or_else(|poison| poison.into_inner()) = Some(Seams { clock, notifier });
}

fn current() -> Option<(Arc<dyn Clock>, Arc<dyn Notifier>)> {
    let guard = seams().lock().unwrap_or_else(|poison| poison.into_inner());
    guard
        .as_ref()
        .map(|seams| (seams.clock.clone(), seams.notifier.clone()))
}

pub fn now_ms() -> u64 {
    current().map(|(clock, _)| clock.now_ms()).unwrap_or(0)
}

fn attempts() -> &'static Mutex<HashMap<(String, String), u32>> {
    static ATTEMPTS: OnceLock<Mutex<HashMap<(String, String), u32>>> = OnceLock::new();
    ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "test-seams")]
pub fn reset_for_test() {
    *seams().lock().unwrap_or_else(|poison| poison.into_inner()) = None;
    attempts()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clear();
}

pub fn notice_text(thread: &str, count: usize) -> String {
    let word = if count == 1 {
        "ny besked"
    } else {
        "nye beskeder"
    };
    format!(
        "[Talminal] {count} {word} i traad {thread}. Kald card_inbox(\"{thread}\") for at laese dem, \
         og svar med card_say i traaden - ikke i din terminal."
    )
}

pub fn deliver(card: &str, thread: &str) -> TickOutcome {
    let Some((clock, notifier)) = current() else {
        return TickOutcome::Nothing;
    };
    let count = super::inbox_len(thread, card);
    let waiting = super::get(thread).is_some_and(|current| current.wake.contains(card));
    if !waiting || count == 0 {
        return TickOutcome::Nothing;
    }
    let key = (card.to_string(), thread.to_string());
    if notifier.is_busy(card) {
        return TickOutcome::Busy;
    }
    match notifier.write_notice(card, &notice_text(thread, count)) {
        Ok(()) => {
            super::clear_wake(card, thread);
            attempts()
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .remove(&key);
            mark_delivered(thread, card, clock.now_ms());
            TickOutcome::Delivered
        }
        Err(error) => {
            eprintln!("threads: wake failed for {card}/{thread}: {error}");
            let attempts_now = {
                let mut counts = attempts()
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                let slot = counts.entry(key.clone()).or_insert(0);
                *slot += 1;
                *slot
            };
            if attempts_now < MAX_WAKE_ATTEMPTS {
                return TickOutcome::Failed {
                    attempts: attempts_now,
                };
            }
            attempts()
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .remove(&key);
            super::close_thread(thread, TerminalReason::DeliveryFailed, Some(card));
            TickOutcome::DeadLettered
        }
    }
}

fn mark_delivered(thread: &str, card: &str, now: u64) {
    let mut map = super::lock_threads();
    let Some(current) = map.get_mut(thread) else {
        return;
    };
    let Some(pending) = current.pending.as_mut() else {
        return;
    };
    if pending.assignee != card || pending.delivered_at_ms.is_some() {
        return;
    }
    pending.delivered_at_ms = Some(now);
    pending.idle_deadline_ms = Some(now + IDLE_MS);
    pending.absolute_deadline_ms = Some(now + ABSOLUTE_MS);
}

pub fn run_once() -> Vec<(String, String, TickOutcome)> {
    super::wakes()
        .into_iter()
        .map(|(card, thread)| {
            let outcome = deliver(&card, &thread);
            (card, thread, outcome)
        })
        .filter(|(_, _, outcome)| *outcome != TickOutcome::Nothing)
        .collect()
}
