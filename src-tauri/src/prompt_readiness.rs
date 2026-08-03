//! Claude Code input-readiness derived from the sequenced raw PTY stream.
//!
//! Startup is ready only after alt-screen entry, Claude's prompt marker, and
//! the blank input cursor (`CSI <row>;3H CSI ?25h`). After prompt text is
//! written, Enter is gated on two newer-reader facts: the normalized prompt
//! text was echoed, then Claude presented an input cursor again. This avoids
//! both a blind sleep and stale output that was read before the write.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::profiles::ReadinessSpec;

const WAITING: u8 = 0;
const READY: u8 = 1;
const CANCELLED: u8 = 2;
const CR_WRITING: u8 = 3;
// TUI-neutral alt-screen-literal (bruges naar spec.require_alt_screen er sat).
const ALT_SCREEN: &[u8] = b"\x1B[?1049h";
// CC's markoerdata (nu ogsaa profiles::CLAUDE_READINESS.live_prompts) —
// kun brugt af testhjaelperne nedenfor (ready_signal m.fl.); FSM-logikken
// laeser fra self.spec.live_prompts.
#[cfg(test)]
const LIVE_PROMPTS: [&[u8]; 2] = [b"\xE2\x9D\xAF\xC2\xA0", b">\xC2\xA0"];
const CURSOR_ENABLE: &[u8] = b"\x1B[?25h";
const MAX_ECHO_PATTERN: usize = 128;

#[derive(Clone, Copy)]
enum EchoEscape {
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
    Dcs,
    DcsEscape,
}

struct Matcher {
    alt_screen_prefix: usize,
    // Én prefix-counter PR. moenster i spec.live_prompts (GPT-review-noten:
    // ikke en rullende hale) — dimensioneret ved konstruktion, saa vilkaarlige
    // moenster-laengder/-antal understoettes pr. profil.
    prompt_prefixes: Vec<usize>,
    anchor_stage: u8,
    cursor_prefix: usize,
    row: u16,
    column: u16,
    generation: u64,
    saw_alt_screen: bool,
    saw_prompt: bool,
    awaiting_text_redraw: bool,
    input_watermark: u64,
    watermark: u64,
    echo_pattern: Vec<u8>,
    echo_failure: Vec<usize>,
    echo_prefix: usize,
    echo_escape: EchoEscape,
    echo_seen: bool,
}

impl Matcher {
    fn new(spec: &'static ReadinessSpec) -> Self {
        Self {
            alt_screen_prefix: 0,
            prompt_prefixes: vec![0; spec.live_prompts.len()],
            anchor_stage: 0,
            cursor_prefix: 0,
            row: 0,
            column: 0,
            generation: 0,
            // Alt-screen-stadiet initialiseres som allerede-set naar profilen
            // ikke kraever det (codex koerer inline-TUI, spike rev 1.2).
            saw_alt_screen: !spec.require_alt_screen,
            saw_prompt: false,
            awaiting_text_redraw: false,
            input_watermark: 0,
            watermark: 0,
            echo_pattern: Vec::new(),
            echo_failure: Vec::new(),
            echo_prefix: 0,
            echo_escape: EchoEscape::Ground,
            echo_seen: false,
        }
    }

    fn advance_literal(prefix: &mut usize, pattern: &[u8], byte: u8) -> bool {
        if byte == pattern[*prefix] {
            *prefix += 1;
            *prefix == pattern.len()
        } else {
            *prefix = usize::from(byte == pattern[0]);
            false
        }
    }

    fn restart_anchor_with(&mut self, byte: u8) {
        self.anchor_stage = u8::from(byte == b'\x1B');
        self.cursor_prefix = 0;
        self.row = 0;
        self.column = 0;
    }

    /// Match `CSI <non-zero row>;<non-zero col>H CSI ?25h`.
    fn observe_input_cursor(&mut self, byte: u8, require_column_three: bool) -> bool {
        match self.anchor_stage {
            0 => {
                if byte == b'\x1B' {
                    self.anchor_stage = 1;
                }
            }
            1 => {
                if byte == b'[' {
                    self.anchor_stage = 2;
                } else {
                    self.restart_anchor_with(byte);
                }
            }
            2 => {
                if matches!(byte, b'1'..=b'9') {
                    self.row = u16::from(byte - b'0');
                    self.anchor_stage = 3;
                } else {
                    self.restart_anchor_with(byte);
                }
            }
            3 => {
                if byte.is_ascii_digit() {
                    self.row = self
                        .row
                        .saturating_mul(10)
                        .saturating_add(u16::from(byte - b'0'));
                } else if byte == b';' {
                    self.anchor_stage = 4;
                } else {
                    self.restart_anchor_with(byte);
                }
            }
            4 => {
                if matches!(byte, b'1'..=b'9') {
                    self.column = u16::from(byte - b'0');
                    self.anchor_stage = 5;
                } else {
                    self.restart_anchor_with(byte);
                }
            }
            5 => {
                if byte.is_ascii_digit() {
                    self.column = self
                        .column
                        .saturating_mul(10)
                        .saturating_add(u16::from(byte - b'0'));
                } else if byte == b'H' && (!require_column_three || self.column == 3) {
                    self.anchor_stage = 6;
                    self.cursor_prefix = 0;
                } else {
                    self.restart_anchor_with(byte);
                }
            }
            6 => {
                if byte == CURSOR_ENABLE[self.cursor_prefix] {
                    self.cursor_prefix += 1;
                    if self.cursor_prefix == CURSOR_ENABLE.len() {
                        return true;
                    }
                } else {
                    self.restart_anchor_with(byte);
                }
            }
            _ => unreachable!(),
        }
        false
    }

    fn normalized_echo_byte(&mut self, byte: u8) -> Option<u8> {
        match self.echo_escape {
            EchoEscape::Ground => {
                if byte == b'\x1B' {
                    self.echo_escape = EchoEscape::Escape;
                    None
                } else if byte.is_ascii_whitespace() || byte.is_ascii_control() {
                    None
                } else {
                    Some(byte)
                }
            }
            EchoEscape::Escape => {
                self.echo_escape = match byte {
                    b'[' => EchoEscape::Csi,
                    b']' => EchoEscape::Osc,
                    b'P' => EchoEscape::Dcs,
                    _ => EchoEscape::Ground,
                };
                None
            }
            EchoEscape::Csi => {
                if matches!(byte, 0x40..=0x7E) {
                    self.echo_escape = EchoEscape::Ground;
                }
                None
            }
            EchoEscape::Osc => {
                if byte == 0x07 {
                    self.echo_escape = EchoEscape::Ground;
                } else if byte == b'\x1B' {
                    self.echo_escape = EchoEscape::OscEscape;
                }
                None
            }
            EchoEscape::OscEscape => {
                self.echo_escape = if byte == b'\\' {
                    EchoEscape::Ground
                } else if byte == b'\x1B' {
                    EchoEscape::OscEscape
                } else {
                    EchoEscape::Osc
                };
                None
            }
            EchoEscape::Dcs => {
                if byte == b'\x1B' {
                    self.echo_escape = EchoEscape::DcsEscape;
                }
                None
            }
            EchoEscape::DcsEscape => {
                self.echo_escape = if byte == b'\\' {
                    EchoEscape::Ground
                } else if byte == b'\x1B' {
                    EchoEscape::DcsEscape
                } else {
                    EchoEscape::Dcs
                };
                None
            }
        }
    }

    fn observe_echo(&mut self, byte: u8) -> bool {
        let Some(byte) = self.normalized_echo_byte(byte) else {
            return false;
        };
        while self.echo_prefix > 0 && byte != self.echo_pattern[self.echo_prefix] {
            self.echo_prefix = self.echo_failure[self.echo_prefix - 1];
        }
        if byte == self.echo_pattern[self.echo_prefix] {
            self.echo_prefix += 1;
            self.echo_prefix == self.echo_pattern.len()
        } else {
            false
        }
    }

    /// Nulstiller ALT prompt-/echo-matchestof. Ét sted, fordi et nyt felt paa
    /// `Matcher` ellers skal huskes i tre uafhaengige nulstillinger — og den
    /// der glemmes lekker state ind i naeste generation.
    fn clear_prompt_match(&mut self) {
        self.prompt_prefixes.fill(0);
        self.anchor_stage = 0;
        self.cursor_prefix = 0;
        self.saw_prompt = false;
        self.awaiting_text_redraw = false;
        self.echo_pattern.clear();
        self.echo_failure.clear();
        self.echo_prefix = 0;
        self.echo_escape = EchoEscape::Ground;
        self.echo_seen = false;
    }

    fn finish_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.clear_prompt_match();
    }

    fn reset_input_match(&mut self) {
        self.clear_prompt_match();
        self.row = 0;
        self.column = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptWait {
    Ready,
    TimedOut,
    Cancelled,
}

/// One instance belongs to exactly one PTY run. Replacing it on respawn is
/// the ABA guard: old reader output can never ready a new run.
pub struct PromptReadiness {
    spec: &'static ReadinessSpec,
    status: AtomicU8,
    armed: AtomicBool,
    published_generation: AtomicU64,
    matched: Mutex<Matcher>,
    submit_lock: Mutex<()>,
    /// Condvar predicate mutex, deliberately separate from `matched`: cancel
    /// never waits behind output parsing or a PTY writer.
    wake: Mutex<()>,
    changed: Condvar,
}

impl PromptReadiness {
    pub fn new(spec: &'static ReadinessSpec) -> Self {
        Self {
            spec,
            status: AtomicU8::new(WAITING),
            armed: AtomicBool::new(false),
            published_generation: AtomicU64::new(0),
            matched: Mutex::new(Matcher::new(spec)),
            submit_lock: Mutex::new(()),
            wake: Mutex::new(()),
            changed: Condvar::new(),
        }
    }

    fn signal_change(&self) {
        let _wake = self
            .wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.changed.notify_all();
    }

    fn publish_input_ready(&self, matched: &mut Matcher) -> bool {
        if self
            .status
            .compare_exchange(WAITING, READY, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        matched.finish_generation();
        self.published_generation
            .store(matched.generation, Ordering::Release);
        self.signal_change();
        true
    }

    /// Feed one post-DSR-filter chunk and the sequence assigned by PtyHost's
    /// reader before callback dispatch.
    pub fn observe(&self, sequence: u64, bytes: &[u8]) {
        let status = self.status.load(Ordering::Acquire);
        if status == CANCELLED
            || status == CR_WRITING
            || (status == READY && !self.armed.load(Ordering::Acquire))
        {
            return;
        }
        let mut matched = self
            .matched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.status.load(Ordering::Acquire) == CANCELLED {
            return;
        }

        let status = self.status.load(Ordering::Acquire);
        if status == WAITING {
            if sequence <= matched.input_watermark {
                return;
            }
            for &byte in bytes {
                if !matched.saw_alt_screen {
                    if Matcher::advance_literal(&mut matched.alt_screen_prefix, ALT_SCREEN, byte) {
                        matched.saw_alt_screen = true;
                    }
                    continue;
                }
                if !matched.saw_prompt {
                    let mut found = false;
                    for (index, pattern) in self.spec.live_prompts.iter().enumerate() {
                        if Matcher::advance_literal(
                            &mut matched.prompt_prefixes[index],
                            pattern,
                            byte,
                        ) {
                            found = true;
                        }
                    }
                    if found {
                        matched.saw_prompt = true;
                        matched.anchor_stage = 0;
                    }
                    continue;
                }
                if matched.observe_input_cursor(byte, self.spec.require_column_three) {
                    self.publish_input_ready(&mut matched);
                    return;
                }
            }
            return;
        }

        if status != READY || !matched.awaiting_text_redraw || sequence <= matched.watermark {
            return;
        }
        for &byte in bytes {
            if !matched.echo_seen {
                if matched.observe_echo(byte) {
                    matched.echo_seen = true;
                    matched.anchor_stage = 0;
                    matched.cursor_prefix = 0;
                }
                continue;
            }
            if matched.observe_input_cursor(byte, false) {
                matched.finish_generation();
                self.published_generation
                    .store(matched.generation, Ordering::Release);
                self.armed.store(false, Ordering::Release);
                self.signal_change();
                return;
            }
        }
    }

    /// Cancellation is independent of the writer and output matcher. The tiny
    /// condvar mutex closes the check-to-park lost-wakeup window, so close/kill
    /// can wake every waiter without waiting behind PTY parsing.
    pub fn cancel(&self) {
        self.status.store(CANCELLED, Ordering::Release);
        self.armed.store(false, Ordering::Release);
        self.signal_change();
    }

    pub fn is_cancelled(&self) -> bool {
        self.status.load(Ordering::Acquire) == CANCELLED
    }

    pub fn lock_submit(&self) -> MutexGuard<'_, ()> {
        self.submit_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn wait(&self, timeout: Duration) -> PromptWait {
        let started = Instant::now();
        let mut wake = self
            .wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            match self.status.load(Ordering::Acquire) {
                READY => return PromptWait::Ready,
                CANCELLED => return PromptWait::Cancelled,
                _ => {}
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return PromptWait::TimedOut;
            }
            let (next, timed_out) = self
                .changed
                .wait_timeout(wake, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            wake = next;
            if timed_out.timed_out()
                && !matches!(self.status.load(Ordering::Acquire), READY | CANCELLED)
            {
                return PromptWait::TimedOut;
            }
        }
    }

    /// Arm post-text detection at the PtyHost reader watermark captured under
    /// its writer lock. The expected text is normalized exactly like PTY echo:
    /// whitespace and ANSI control traffic do not participate in the match.
    pub fn arm_text_redraw(&self, watermark: u64, text: &str) -> Option<u64> {
        let pattern: Vec<u8> = text
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace() && !byte.is_ascii_control())
            .take(MAX_ECHO_PATTERN)
            .collect();
        if pattern.is_empty() {
            return None;
        }
        let mut matched = self
            .matched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.status.load(Ordering::Acquire) != READY || matched.generation == 0 {
            return None;
        }
        let generation = matched.generation;
        matched.awaiting_text_redraw = true;
        matched.watermark = watermark;
        matched.echo_pattern = pattern;
        matched.echo_failure = vec![0; matched.echo_pattern.len()];
        for index in 1..matched.echo_pattern.len() {
            let mut candidate = matched.echo_failure[index - 1];
            while candidate > 0 && matched.echo_pattern[index] != matched.echo_pattern[candidate] {
                candidate = matched.echo_failure[candidate - 1];
            }
            if matched.echo_pattern[index] == matched.echo_pattern[candidate] {
                candidate += 1;
            }
            matched.echo_failure[index] = candidate;
        }
        matched.echo_prefix = 0;
        matched.echo_escape = EchoEscape::Ground;
        matched.echo_seen = false;
        matched.anchor_stage = 0;
        matched.cursor_prefix = 0;
        self.armed.store(true, Ordering::Release);
        Some(generation)
    }

    pub fn disarm_text_redraw(&self, generation: u64) {
        let mut matched = self
            .matched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matched.generation == generation && matched.awaiting_text_redraw {
            matched.awaiting_text_redraw = false;
            matched.echo_pattern.clear();
            matched.echo_failure.clear();
            matched.echo_prefix = 0;
            matched.echo_seen = false;
            self.armed.store(false, Ordering::Release);
        }
    }

    /// Linearization point immediately before the carriage return write.
    /// Reader output is conservatively ignored until `mark_submitted`, so a
    /// delayed typed-input redraw cannot re-ready a queued submit before CR.
    pub fn prepare_submission(&self) -> bool {
        let mut matched = self
            .matched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self
            .status
            .compare_exchange(READY, CR_WRITING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        matched.reset_input_match();
        self.armed.store(false, Ordering::Release);
        true
    }

    /// Start the fresh-input wait after CR has been flushed. Any output that
    /// raced entirely inside the local write is discarded (fail-closed); a
    /// later prompt must carry a sequence beyond this watermark.
    pub fn mark_submitted(&self, output_watermark: u64) -> bool {
        let mut matched = self
            .matched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self
            .status
            .compare_exchange(CR_WRITING, WAITING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        matched.input_watermark = output_watermark;
        true
    }

    pub fn wait_for_generation_after(&self, generation: u64, timeout: Duration) -> PromptWait {
        let started = Instant::now();
        let mut wake = self
            .wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if self.is_cancelled() {
                return PromptWait::Cancelled;
            }
            if self.published_generation.load(Ordering::Acquire) > generation {
                return PromptWait::Ready;
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return PromptWait::TimedOut;
            }
            let (next, timed_out) = self
                .changed
                .wait_timeout(wake, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            wake = next;
            if timed_out.timed_out()
                && !self.is_cancelled()
                && self.published_generation.load(Ordering::Acquire) <= generation
            {
                return PromptWait::TimedOut;
            }
        }
    }

    /// Preserve the configured minimum submit gap while allowing cancellation
    /// to wake the wait immediately.
    pub fn wait_gap_unless_cancelled(&self, gap: Duration) -> bool {
        let started = Instant::now();
        let mut wake = self
            .wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if self.is_cancelled() {
                return false;
            }
            let remaining = gap.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return true;
            }
            let (next, timed_out) = self
                .changed
                .wait_timeout(wake, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            wake = next;
            if timed_out.timed_out() {
                return !self.is_cancelled();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PromptReadiness, PromptWait, ALT_SCREEN, CURSOR_ENABLE, LIVE_PROMPTS};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    fn ready_signal(marker: &[u8], row: u16) -> Vec<u8> {
        [
            ALT_SCREEN,
            marker,
            b"Try refactor this file",
            format!("\x1B[{row};3H").as_bytes(),
            CURSOR_ENABLE,
        ]
        .concat()
    }

    fn next_prompt_signal() -> Vec<u8> {
        [LIVE_PROMPTS[1], b"Try another task\x1B[28;3H\x1B[?25h"].concat()
    }

    fn make_ready(readiness: &PromptReadiness) {
        readiness.observe(1, &ready_signal(LIVE_PROMPTS[1], 27));
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::Ready);
    }

    fn assert_detects_every_split(marker: &[u8], row: u16) {
        let signal = ready_signal(marker, row);
        for split in 0..=signal.len() {
            let readiness = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
            readiness.observe(1, &signal[..split]);
            if split < signal.len() {
                assert_eq!(readiness.wait(Duration::ZERO), PromptWait::TimedOut);
            }
            readiness.observe(2, &signal[split..]);
            assert_eq!(readiness.wait(Duration::ZERO), PromptWait::Ready);
        }
    }

    #[test]
    fn detects_both_captured_prompt_versions_across_every_chunk_boundary() {
        for (marker, row) in LIVE_PROMPTS.into_iter().zip([32, 27]) {
            assert_detects_every_split(marker, row);
        }
    }

    #[test]
    fn history_placeholder_and_cursor_without_input_anchor_are_not_ready() {
        let readiness = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
        readiness.observe(1, ALT_SCREEN);
        readiness.observe(
            2,
            b"history: \xE2\x9D\xAF old\r\n> Continue\r\n\x1B[27;3H\x1B[?25h",
        );
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::TimedOut);
        readiness.observe(3, LIVE_PROMPTS[1]);
        readiness.observe(4, b"Try fix\x1B[27;4H\x1B[?25h");
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::TimedOut);
        readiness.observe(5, b"\x1B[27;3H\x1B[?25h");
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::Ready);
    }

    #[test]
    fn captured_v212_and_v216_excerpts_require_post_prompt_column_three() {
        for capture in [
            b"\x1B[?25h\x1B[?1049h\x1B[?25l>\xC2\xA0Try refactor\x1B[27;3H\x1B[?25h".as_slice(),
            b"\x1B[?1049hhistory: \xE2\x9D\xAF old\r\n\xE2\x9D\xAF\xC2\xA0Try fix\x1B[32;3H\x1B[?25h".as_slice(),
        ] {
            let readiness = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
            for (index, chunk) in capture.chunks(7).enumerate() {
                readiness.observe(index as u64 + 1, chunk);
            }
            assert_eq!(readiness.wait(Duration::ZERO), PromptWait::Ready);
        }
    }

    #[test]
    fn post_text_gate_requires_new_sequence_full_echo_then_cursor() {
        let readiness = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
        make_ready(&readiness);
        let text = "P5_SENTINEL: Reply with exactly ACK_P5_SENTINEL.";
        let generation = readiness.arm_text_redraw(10, text).unwrap();

        // A delayed pre-write chunk contains perfect-looking evidence but has
        // the watermark sequence and is therefore ignored.
        readiness.observe(
            10,
            b"P5_SENTINEL: Reply with exactly ACK_P5_SENTINEL.\x1B[26;52H\x1B[?25h",
        );
        assert_eq!(
            readiness.wait_for_generation_after(generation, Duration::ZERO),
            PromptWait::TimedOut
        );
        // Resize/status cursors after the watermark are not sufficient.
        readiness.observe(11, b"\x1B[28;3H\x1B[?25h\x1B[4;77H\x1B[?25h");
        assert_eq!(
            readiness.wait_for_generation_after(generation, Duration::ZERO),
            PromptWait::TimedOut
        );
        readiness.observe(
            12,
            b"P5_SENTINEL:\x1B[1CReply\x1B[1Cwith\x1B[1Cexactly\x1B[1CACK_P5_SENTINEL.",
        );
        assert_eq!(
            readiness.wait_for_generation_after(generation, Duration::ZERO),
            PromptWait::TimedOut
        );
        readiness.observe(13, b"\x1B[26;52H\x1B[?25h");
        assert_eq!(
            readiness.wait_for_generation_after(generation, Duration::ZERO),
            PromptWait::Ready
        );
    }

    #[test]
    fn submitted_prompt_requires_a_newer_fresh_input_prompt() {
        let readiness = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
        make_ready(&readiness);
        let generation = readiness.arm_text_redraw(1, "hello").unwrap();
        readiness.observe(2, b"hello\x1B[27;8H\x1B[?25h");
        assert_eq!(
            readiness.wait_for_generation_after(generation, Duration::ZERO),
            PromptWait::Ready
        );

        assert!(readiness.prepare_submission());
        assert!(!readiness.prepare_submission());
        readiness.observe(20, &next_prompt_signal());
        assert!(readiness.mark_submitted(20));
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::TimedOut);

        let next_prompt = next_prompt_signal();
        readiness.observe(20, &next_prompt);
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::TimedOut);
        readiness.observe(21, &next_prompt);
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::Ready);
    }

    #[test]
    fn cancellation_wins_over_post_write_wait_transition() {
        let readiness = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
        make_ready(&readiness);
        assert!(readiness.prepare_submission());
        readiness.cancel();
        assert!(!readiness.mark_submitted(2));
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::Cancelled);
    }

    #[test]
    fn cancellation_cannot_be_resurrected_by_a_ready_publish() {
        let readiness = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
        readiness.cancel();
        let mut matched = readiness.matched.lock().unwrap();
        assert!(!readiness.publish_input_ready(&mut matched));
        drop(matched);
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::Cancelled);
    }

    #[test]
    fn echo_matcher_handles_overlapping_prefixes_across_chunks() {
        let readiness = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
        make_ready(&readiness);
        let generation = readiness.arm_text_redraw(1, "aab").unwrap();
        readiness.observe(2, b"aa");
        readiness.observe(3, b"ab\x1B[27;6H\x1B[?25h");
        assert_eq!(
            readiness.wait_for_generation_after(generation, Duration::ZERO),
            PromptWait::Ready
        );
    }

    #[test]
    fn cancel_is_bounded_even_while_matcher_is_locked() {
        let readiness = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
        let _guard = readiness.matched.lock().unwrap();
        let started = Instant::now();
        readiness.cancel();
        assert!(started.elapsed() < Duration::from_millis(20));
        assert!(readiness.is_cancelled());
    }

    #[test]
    fn cancel_wakes_startup_and_redraw_waiters() {
        let readiness = Arc::new(PromptReadiness::new(&crate::profiles::CLAUDE_READINESS));
        let waiter = Arc::clone(&readiness);
        let (result_tx, result_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            result_tx
                .send(waiter.wait(Duration::from_secs(10)))
                .unwrap();
        });
        readiness.cancel();
        assert_eq!(
            result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            PromptWait::Cancelled
        );
        thread.join().unwrap();

        let readiness = Arc::new(PromptReadiness::new(&crate::profiles::CLAUDE_READINESS));
        make_ready(&readiness);
        let generation = readiness.arm_text_redraw(1, "hello").unwrap();
        let waiter = Arc::clone(&readiness);
        let (result_tx, result_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            result_tx
                .send(waiter.wait_for_generation_after(generation, Duration::from_secs(10)))
                .unwrap();
        });
        readiness.cancel();
        assert_eq!(
            result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            PromptWait::Cancelled
        );
        thread.join().unwrap();
    }

    #[test]
    fn codex_boot_reaches_ready_without_alt_screen() {
        // Spike-uddrag (codex-idle.raw @7250-7560, forkortet): live-markør + CUP kol 3 + cursor-vis
        let r = PromptReadiness::new(&crate::profiles::CODEX_READINESS);
        let boot: &[u8] = b"\x1b[?2026h\x1b[K\r\n\x1b[K\x1b[1m\r\n\xE2\x80\xBA\x1b[22m \x1b[2mFind and fix a bug\x1b[16;3H\x1b[?25h\x1b[?2026l";
        r.observe(1, boot);
        assert!(matches!(
            r.wait(Duration::from_millis(50)),
            PromptWait::Ready
        ));
    }

    #[test]
    fn codex_dim_shutdown_marker_does_not_arm() {
        let r = PromptReadiness::new(&crate::profiles::CODEX_READINESS);
        // dim-varianten (ESC[2m) er shutdown — maa IKKE taelle som live prompt
        let bytes: &[u8] =
            b"\x1b[2m\r\n\xE2\x80\xBA\x1b[22m \x1b[2mShutting down...\x1b[16;3H\x1b[?25h";
        r.observe(1, bytes);
        assert!(matches!(
            r.wait(Duration::from_millis(50)),
            PromptWait::TimedOut
        ));
    }

    #[test]
    fn claude_still_requires_alt_screen() {
        let r = PromptReadiness::new(&crate::profiles::CLAUDE_READINESS);
        // prompt + cursor UDEN forudgaaende alt-screen: maa ikke blive Ready
        let bytes: &[u8] = b"\xE2\x9D\xAF\xC2\xA0\x1b[5;3H\x1b[?25h";
        r.observe(1, bytes);
        assert!(matches!(
            r.wait(Duration::from_millis(50)),
            PromptWait::TimedOut
        ));
    }

    #[test]
    fn codex_chunk_split_boot_bytes_reach_ready_one_byte_at_a_time() {
        // GPT-review-robusthedstest (a): markør-counteren maa ikke kraeve
        // mønstret i ét chunk — samme boot-bytes leveret 1 byte ad gangen.
        let r = PromptReadiness::new(&crate::profiles::CODEX_READINESS);
        let boot: &[u8] = b"\x1b[?2026h\x1b[K\r\n\x1b[K\x1b[1m\r\n\xE2\x80\xBA\x1b[22m \x1b[2mFind and fix a bug\x1b[16;3H\x1b[?25h\x1b[?2026l";
        for (index, &byte) in boot.iter().enumerate() {
            r.observe(index as u64 + 1, &[byte]);
        }
        assert!(matches!(
            r.wait(Duration::from_millis(50)),
            PromptWait::Ready
        ));
    }

    fn codex_ready_signal(row: u16) -> Vec<u8> {
        [
            b"\x1b[1m\r\n\xE2\x80\xBA".as_slice(),
            b" Find and fix a bug",
            format!("\x1B[{row};3H").as_bytes(),
            CURSOR_ENABLE,
        ]
        .concat()
    }

    fn make_codex_ready(readiness: &PromptReadiness) {
        readiness.observe(1, &codex_ready_signal(16));
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::Ready);
    }

    #[test]
    fn codex_post_text_gate_reaches_ready_after_echo_and_cursor() {
        // GPT-review-robusthedstest (b): spejl af CC-generationstesten
        // (submitted_prompt_requires_a_newer_fresh_input_prompt) med codex-
        // spec og codex-markør-bytes.
        let readiness = PromptReadiness::new(&crate::profiles::CODEX_READINESS);
        make_codex_ready(&readiness);
        let generation = readiness.arm_text_redraw(1, "hej").unwrap();
        readiness.observe(2, b"hej\x1B[16;7H\x1B[?25h");
        assert_eq!(
            readiness.wait_for_generation_after(generation, Duration::ZERO),
            PromptWait::Ready
        );
    }

    #[test]
    fn cancellation_interrupts_submit_gap_and_prevents_rearming() {
        let readiness = Arc::new(PromptReadiness::new(&crate::profiles::CLAUDE_READINESS));
        make_ready(&readiness);
        let waiter = Arc::clone(&readiness);
        let (result_tx, result_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            result_tx
                .send(waiter.wait_gap_unless_cancelled(Duration::from_secs(10)))
                .unwrap();
        });
        readiness.cancel();
        assert!(!result_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        assert!(readiness.arm_text_redraw(1, "late").is_none());
        readiness.observe(2, &ready_signal(LIVE_PROMPTS[0], 32));
        assert_eq!(readiness.wait(Duration::ZERO), PromptWait::Cancelled);
        thread.join().unwrap();
    }
}
