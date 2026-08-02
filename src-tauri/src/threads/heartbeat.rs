//! Appens hjerteslag for traade. Raekkefoelgen (levér foer du doemmer) og
//! forholdet mellem leverings- og sweep-frekvens er BESLUTNINGER og bor derfor
//! her, ikke i main.rs' timer — den maa kun levere tid og emit.

use std::collections::HashMap;

use super::{dispatch, sweep};

/// Sweeperen koerer paa hvert N'te slag. Deadlines er minutter; at doemme dem
/// fire gange i sekundet er ren spild.
pub const SWEEP_EVERY: u32 = 20;

#[derive(Default)]
pub struct ChangeTracker {
    seen: HashMap<String, u64>,
    beats: u32,
}

pub struct Beat {
    /// Traade hvis beskedliste er vokset siden sidste slag — dem og kun dem
    /// skal `chat-thread-updated` emittes for.
    pub changed: Vec<String>,
    pub delivered: usize,
}

pub fn beat(tracker: &mut ChangeTracker) -> Beat {
    // Levér FOER du doemmer: en notits der netop blev leveret skal have sat
    // urene, inden sweeperen ser paa dem.
    let delivered = dispatch::run_once()
        .iter()
        .filter(|(_, _, outcome)| *outcome == dispatch::TickOutcome::Delivered)
        .count();

    // Taelleren laeses FOER den taelles op, saa slag 1 er et sweep og de naeste
    // N-1 ikke er. Foerste slag koster intet (kortet er lige leveret, ingen
    // frist kan vaere overskredet), og til gengaeld er der ingen tavs periode
    // ved opstart hvor en gammel frist ligger og venter paa slag N.
    let due = tracker.beats % SWEEP_EVERY == 0;
    tracker.beats = tracker.beats.wrapping_add(1);
    if due {
        sweep::sweep();
    }

    let counts = super::message_counts();
    let mut changed = Vec::new();
    for (thread, count) in counts {
        if tracker.seen.get(&thread).copied() != Some(count) {
            tracker.seen.insert(thread.clone(), count);
            changed.push(thread);
        }
    }
    changed.sort();
    Beat { changed, delivered }
}
