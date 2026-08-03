//! Traade: agent-til-agent-kanalen (spec 2026-07-25-agent-til-agent-chatkort-design.md rev 1.2).
//!
//! Modulet bor i LIB-craten med vilje: hele beslutningslogikken skal kunne naas
//! fra `tests/`. Kun Tauri-glue og de aegte seam-implementationer bor i main.rs.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

pub mod archive;
pub mod dispatch;
pub mod heartbeat;
pub mod ops;
pub mod pair;
pub mod policy;
pub mod sweep;

/// Opstartens seed af traad-taelleren. Genudstillet paa modul-roden fordi det
/// er et opstarts-skridt paa linje med `terminalize_awaiting_on_startup` og
/// ikke en del af parringens flade — kaldes fra main.rs foer foerste parring.
pub use pair::seed_counter_from_disk;

pub const MAX_HOPS: u32 = 20;
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;
pub const MAX_PURPOSE_CHARS: usize = 60;
pub const INBOX_BATCH_MAX: usize = 5;

/// Reserverede afsender-id'er. De er IKKE medlemmer af traaden (medlemsmodellen,
/// spec §3.1) og kan ikke kollidere med registryets `card-N`-navne.
pub const OWNER: &str = "owner";
pub const SYSTEM: &str = "system";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FromKind {
    Agent,
    Human,
    System,
}

impl FromKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FromKind::Agent => "agent",
            FromKind::Human => "human",
            FromKind::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Sparring,
    Delegation,
    Answer,
    Status,
}

impl Intent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Intent::Sparring => "sparring",
            Intent::Delegation => "delegation",
            Intent::Answer => "answer",
            Intent::Status => "status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Open,
    Awaiting,
    Closed,
}

impl ThreadState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThreadState::Open => "open",
            ThreadState::Awaiting => "awaiting",
            ThreadState::Closed => "closed",
        }
    }
}

/// Praecis EEN af disse afslutter en udestaaende delegering. `peer_active` er
/// A9's fjerde klasse ("hard-cap men modparten arbejdede faktisk") og kan ikke
/// udtrykkes af enum'en alene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalReason {
    /// Bemaerk: `Answer` lukker DELEGERINGEN og haandteres inde i `post()`.
    /// Den naar aldrig `close_with_guard` — hverken gennem `close_thread`
    /// eller `close_thread_if` (spec §4.1 rev 1.2).
    Answer,
    IdleTimeout {
        peer_active: bool,
    },
    AbsoluteTimeout {
        peer_active: bool,
    },
    BackstopCleanup,
    DeliveryFailed,
    ParticipantLost,
    OwnerStopped,
    RestartAbort,
    HopLimit,
}

#[derive(Debug, Clone)]
pub struct Pending {
    pub request_seq: u64,
    pub requester: String,
    pub assignee: String,
    pub request_ts_ms: u64,
    pub delivered_at_ms: Option<u64>,
    pub idle_deadline_ms: Option<u64>,
    pub absolute_deadline_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub seq: u64,
    pub thread: String,
    pub from_card: String,
    pub from_kind: FromKind,
    pub intent: Intent,
    pub hop: u32,
    pub text: String,
    pub ts_ms: u64,
}

#[derive(Debug, Clone)]
pub struct InboxBatch {
    pub messages: Vec<Message>,
    pub has_more: bool,
    /// Hoejeste `seq` i batchen, eller nul naar batchen er tom.
    pub batch_id: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadView {
    pub state: &'static str,
    pub purpose: String,
    pub hops_used: u32,
    pub hops_left: u32,
    pub messages: Vec<MessageView>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MessageView {
    pub seq: u64,
    pub from_card: String,
    pub from_kind: &'static str,
    pub intent: &'static str,
    pub text: String,
    pub ts_ms: u64,
}

#[derive(Debug, Clone)]
pub struct Thread {
    pub id: String,
    pub purpose: String,
    /// IMMUTABEL efter `create_thread`.
    pub members: Vec<String>,
    pub state: ThreadState,
    pub hops_used: u32,
    pub pending: Option<Pending>,
    pub messages: Vec<Message>,
    pub inbox: HashMap<String, Vec<u64>>,
    pub wake: BTreeSet<String>,
    pub created_at_ms: u64,
}

fn threads_map() -> &'static Mutex<HashMap<String, Thread>> {
    static MAP: OnceLock<Mutex<HashMap<String, Thread>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub type ActivityProbe = Box<dyn Fn(&str) -> bool + Send + Sync>;

fn activity_probe() -> &'static Mutex<Option<ActivityProbe>> {
    static PROBE: OnceLock<Mutex<Option<ActivityProbe>>> = OnceLock::new();
    PROBE.get_or_init(|| Mutex::new(None))
}

pub fn set_activity_probe(probe: ActivityProbe) {
    *activity_probe()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(probe);
}

/// Kaldes aldrig med traad-laasen holdt.
pub fn peer_active(card: &str) -> bool {
    let probe = activity_probe()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    probe.as_ref().map(|probe| probe(card)).unwrap_or(false)
}

#[cfg(feature = "test-seams")]
pub fn set_activity_probe_for_test(active: bool) {
    set_activity_probe(Box::new(move |_| active));
}

#[cfg(feature = "test-seams")]
pub fn reset_activity_probe_for_test() {
    *activity_probe()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = None;
}

pub(crate) fn lock_threads() -> std::sync::MutexGuard<'static, HashMap<String, Thread>> {
    threads_map().lock().unwrap_or_else(|p| p.into_inner())
}

/// KUN til rollback i `card_pair`. Skriver intet terminalt udfald, fordi
/// traaden aldrig blev synlig - brug ALDRIG denne som almindelig lukkevej.
pub(crate) fn drop_thread_for_rollback(id: &str) {
    lock_threads().remove(id);
}

#[cfg(feature = "test-seams")]
pub fn all_thread_ids_for_test() -> Vec<String> {
    lock_threads().keys().cloned().collect()
}

#[cfg(feature = "test-seams")]
pub fn reset_for_test() {
    lock_threads().clear();
}

pub fn sanitize_purpose(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(MAX_PURPOSE_CHARS)
        .collect()
}

pub fn create_thread(id: &str, purpose: &str, members: Vec<String>) -> Result<(), String> {
    if members.len() != 2 {
        return Err(format!(
            "a thread needs exactly two agent members, got {}",
            members.len()
        ));
    }
    if members[0] == members[1] {
        return Err("the two members must be distinct cards".to_string());
    }
    if members.iter().any(|m| m == OWNER || m == SYSTEM) {
        return Err(format!(
            "{OWNER}/{SYSTEM} are reserved sender ids, not members"
        ));
    }
    let mut map = lock_threads();
    if map.contains_key(id) {
        return Err(format!("thread already exists: {id}"));
    }
    map.insert(
        id.to_string(),
        Thread {
            id: id.to_string(),
            purpose: sanitize_purpose(purpose),
            members,
            state: ThreadState::Open,
            hops_used: 0,
            pending: None,
            messages: Vec::new(),
            inbox: HashMap::new(),
            wake: BTreeSet::new(),
            created_at_ms: now_ms(),
        },
    );
    Ok(())
}

pub fn get(id: &str) -> Option<Thread> {
    lock_threads().get(id).cloned()
}

pub fn members_of(id: &str) -> Result<Vec<String>, String> {
    lock_threads()
        .get(id)
        .map(|t| t.members.clone())
        .ok_or_else(|| format!("unknown thread: {id}"))
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct PostRequest {
    pub thread: String,
    pub from_card: String,
    pub from_kind: FromKind,
    pub intent: Intent,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostAccepted {
    pub seq: u64,
    pub hop: u32,
    pub hops_left: u32,
}

enum Verdict {
    Rejected(String),
    HopLimit,
    Accepted { seq: u64, hop: u32, line: String },
}

/// Chokepointet: validerer, appender og koeber en wake uden at vente.
pub fn post(req: PostRequest) -> Result<PostAccepted, String> {
    if req.text.len() > MAX_MESSAGE_BYTES {
        return Err(format!(
            "beskeden er for lang ({} bytes, loft {MAX_MESSAGE_BYTES}) - opsummer",
            req.text.len()
        ));
    }

    let members = members_of(&req.thread)?;
    let recipients: Vec<String> = members
        .iter()
        .filter(|m| *m != &req.from_card)
        .cloned()
        .collect();

    if req.from_kind == FromKind::Agent {
        if !members.iter().any(|m| m == &req.from_card) {
            return Err(format!(
                "{} is not a member of {}",
                req.from_card, req.thread
            ));
        }
        if recipients.is_empty() {
            return Err(format!("self send rejected in {}", req.thread));
        }
        for recipient in &recipients {
            if !policy::read(recipient).allows(&req.from_card) {
                return Err(format!(
                    "accepts_from blocked {} -> {} in {}",
                    req.from_card, recipient, req.thread
                ));
            }
        }
    }

    let verdict = {
        let mut map = lock_threads();
        match map.get_mut(&req.thread) {
            None => Verdict::Rejected(format!("unknown thread: {}", req.thread)),
            Some(t) => decide_and_apply(t, &req, &recipients),
        }
    };

    match verdict {
        Verdict::Rejected(e) => Err(e),
        Verdict::HopLimit => {
            close_thread(&req.thread, TerminalReason::HopLimit, None);
            Err(format!(
                "hop limit {MAX_HOPS} reached in {} - thread closed",
                req.thread
            ))
        }
        Verdict::Accepted { seq, hop, line } => {
            archive::append_lines(&req.thread, &[line]);
            Ok(PostAccepted {
                seq,
                hop,
                hops_left: MAX_HOPS.saturating_sub(hop),
            })
        }
    }
}

/// Hele den tilstandsafhaengige beslutning og mutation under traadlaasen.
fn decide_and_apply(t: &mut Thread, req: &PostRequest, recipients: &[String]) -> Verdict {
    if t.state == ThreadState::Closed {
        return Verdict::Rejected(format!("thread is closed: {}", t.id));
    }
    let is_agent = req.from_kind == FromKind::Agent;
    if req.intent == Intent::Delegation {
        if !is_agent {
            return Verdict::Rejected(format!(
                "only an agent may delegate in {} - the owner writes sparring or status",
                t.id
            ));
        }
        if t.state == ThreadState::Awaiting {
            return Verdict::Rejected(format!(
                "thread {} is awaiting an answer - only one pending delegation at a time",
                t.id
            ));
        }
    }
    let answers_pending = req.intent == Intent::Answer
        && t.pending
            .as_ref()
            .is_some_and(|p| p.assignee == req.from_card);

    if is_agent && t.hops_used >= MAX_HOPS && !answers_pending {
        return Verdict::HopLimit;
    }

    let hop = if is_agent {
        t.hops_used += 1;
        t.hops_used
    } else {
        t.hops_used
    };
    let seq = t.messages.len() as u64 + 1;
    let now = now_ms();
    let msg = Message {
        seq,
        thread: t.id.clone(),
        from_card: req.from_card.clone(),
        from_kind: req.from_kind,
        intent: req.intent,
        hop,
        text: req.text.clone(),
        ts_ms: now,
    };
    let line = archive::line_for(&msg, None);
    t.messages.push(msg);

    for recipient in recipients {
        t.inbox.entry(recipient.clone()).or_default().push(seq);
        t.wake.insert(recipient.clone());
    }

    match req.intent {
        Intent::Delegation => {
            let assignee = recipients.first().cloned().unwrap_or_default();
            t.state = ThreadState::Awaiting;
            t.pending = Some(Pending {
                request_seq: seq,
                requester: req.from_card.clone(),
                assignee,
                request_ts_ms: now,
                delivered_at_ms: None,
                idle_deadline_ms: None,
                absolute_deadline_ms: None,
            });
        }
        Intent::Answer if answers_pending => {
            t.pending = None;
            t.state = ThreadState::Open;
        }
        _ => {}
    }
    Verdict::Accepted { seq, hop, line }
}

/// Den UBETINGEDE af de to offentlige terminalindgange. Idempotent.
///
/// De otte lukkende aarsager har to indgange — denne og den seq-betingede
/// `close_thread_if` — men kun ÉN faktisk vej: begge er tynde skaller om
/// `close_with_guard`. Skal en lukning opfoere sig anderledes, er det dér den
/// aendres; leder du efter alle terminalveje, er det dén ene du skal finde.
pub fn close_thread(id: &str, reason: TerminalReason, lost: Option<&str>) -> bool {
    close_with_guard(id, reason, lost, None)
}

/// Betinget lukkevej: lukker KUN hvis traaden stadig staar i praecis den
/// delegering kalderen saa — `Awaiting` med `pending.request_seq == expect_seq`.
///
/// Sweeperen bygger sit snapshot under traadlaasen, slipper den, maaler derefter
/// modparten og lukker. I det vindue kan svaret naa frem: `post()` saetter
/// `pending = None` og traaden tilbage til `Open`, og en ubetinget lukning ville
/// bagefter rydde `inbox` og `wake` — svaret var altsaa i huset, men leveringen
/// gik tabt, og ejeren fik den usande besked at modparten ikke naaede at svare.
/// `close_thread`s idempotens-check fanger ikke dét, for traaden er hverken
/// lukket eller pending paa det tidspunkt.
///
/// At der sammenlignes paa `request_seq` og ikke kun paa tilstanden er
/// bevidst: bliver svaret fulgt af en NY delegering i samme vindue, staar
/// traaden `Awaiting` igen — men det er en anden delegering end den der loeb
/// toer for tid, og den har sine egne frister.
pub fn close_thread_if(
    id: &str,
    reason: TerminalReason,
    lost: Option<&str>,
    expect_seq: u64,
) -> bool {
    close_with_guard(id, reason, lost, Some(expect_seq))
}

/// Den eneste faktiske terminalvej — begge offentlige indgange lander her.
/// `expect_seq: None` = `close_thread`s uaendrede idempotens-semantik;
/// `Some(seq)` = `close_thread_if`s seq-betingelse.
fn close_with_guard(
    id: &str,
    reason: TerminalReason,
    lost: Option<&str>,
    expect_seq: Option<u64>,
) -> bool {
    debug_assert!(
        reason != TerminalReason::Answer,
        "answer lukker delegeringen inde i post(), ikke traaden"
    );
    let (line, members) = {
        let mut map = lock_threads();
        let Some(t) = map.get_mut(id) else {
            return false;
        };
        match expect_seq {
            // Taberen af kaploebet gaar hjem uden at roere noget.
            Some(expected) => {
                if t.state != ThreadState::Awaiting
                    || t.pending.as_ref().map(|p| p.request_seq) != Some(expected)
                {
                    return false;
                }
            }
            None => {
                if t.state == ThreadState::Closed && t.pending.is_none() {
                    return false;
                }
            }
        }
        let pending = t.pending.take();
        t.state = ThreadState::Closed;

        t.inbox.clear();
        t.wake.clear();

        let assignee = pending
            .as_ref()
            .map(|p| p.assignee.clone())
            .unwrap_or_default();
        let text = terminal_text(
            reason,
            if assignee.is_empty() {
                "modparten"
            } else {
                &assignee
            },
        );
        let seq = t.messages.len() as u64 + 1;
        let msg = Message {
            seq,
            thread: id.to_string(),
            from_card: SYSTEM.to_string(),
            from_kind: FromKind::System,
            intent: Intent::Status,
            hop: t.hops_used,
            text,
            ts_ms: now_ms(),
        };
        let line = archive::line_for(&msg, Some(reason));
        t.messages.push(msg);
        let members = t.members.clone();
        for m in &members {
            if Some(m.as_str()) == lost {
                continue;
            }
            t.inbox.entry(m.clone()).or_default().push(seq);
            t.wake.insert(m.clone());
        }
        (line, members)
    };
    archive::append_lines(id, &[line]);
    if members.len() == 2 {
        policy::revoke(&members[0], &members[1]);
    }
    true
}

/// Et kort er vaek — lukket, doedt eller crashet. Returnerer antallet af traade
/// der blev lukket af netop dette kald.
///
/// `chat_thread` er sat hvis kortet var et CHAT-kort: da er det ejeren der
/// stopper samarbejdet, og ingen agent er "tabt". Ellers var det et deltagende
/// terminal-kort, og dets traade lukkes som `participant_lost` med kortet selv
/// som `lost`, saa udfaldet ikke lander i køen hos et kort der ikke findes.
///
/// Kaldes ALTID uden kort- eller registry-laase (laaseorden §5.6).
pub fn on_card_gone(card: &str, chat_thread: Option<&str>) -> usize {
    if let Some(thread) = chat_thread {
        return usize::from(close_thread(thread, TerminalReason::OwnerStopped, None));
    }
    threads_with_member(card)
        .into_iter()
        .filter(|t| close_thread(t, TerminalReason::ParticipantLost, Some(card)))
        .count()
}

pub fn terminal_text(reason: TerminalReason, assignee: &str) -> String {
    match reason {
        TerminalReason::Answer => format!("{assignee} svarede."),
        TerminalReason::IdleTimeout { .. } => {
            format!("{assignee} var stille i 5 minutter. Opgaven blev ikke udfoert.")
        }
        TerminalReason::AbsoluteTimeout { peer_active: true } => format!(
            "{assignee} naaede den absolutte frist paa 20 minutter, men arbejdede stadig. \
             Opgaven blev afbrudt, ikke opgivet af modparten."
        ),
        TerminalReason::AbsoluteTimeout { peer_active: false } => {
            format!("{assignee} naaede den absolutte frist paa 20 minutter uden aktivitet.")
        }
        TerminalReason::BackstopCleanup => {
            "Delegeringen blev ryddet af backstoppen - en frist blev aldrig ryddet normalt.".into()
        }
        TerminalReason::DeliveryFailed => format!(
            "Beskeden kunne ikke leveres til {assignee} efter 2 forsoeg. Opgaven blev ikke udfoert."
        ),
        TerminalReason::ParticipantLost => {
            format!("{assignee} er vaek. Opgaven blev ikke udfoert.")
        }
        TerminalReason::OwnerStopped => "Du stoppede samarbejdet.".into(),
        TerminalReason::RestartAbort => {
            "Appen blev genstartet mens delegeringen var udestaaende.".into()
        }
        TerminalReason::HopLimit => {
            format!("Traaden naaede loftet paa {MAX_HOPS} beskeder. Opgaven blev ikke afsluttet.")
        }
    }
}

pub fn inbox_len(thread: &str, card: &str) -> usize {
    lock_threads()
        .get(thread)
        .and_then(|t| t.inbox.get(card).map(|v| v.len()))
        .unwrap_or(0)
}

/// Kvitterer foerst for den forrige batch og returnerer derefter op til fem
/// beskeder. Uden ack genleveres samme batch.
pub fn inbox_take(
    thread: &str,
    card: &str,
    ack_through: Option<u64>,
) -> Result<InboxBatch, String> {
    let mut map = lock_threads();
    let current = map
        .get_mut(thread)
        .ok_or_else(|| format!("unknown thread: {thread}"))?;
    if !current.members.iter().any(|member| member == card) {
        return Err(format!("{card} is not a member of {thread}"));
    }
    if let Some(through) = ack_through {
        if let Some(queue) = current.inbox.get_mut(card) {
            queue.retain(|seq| *seq > through);
        }
    }
    let queued = current.inbox.get(card).cloned().unwrap_or_default();
    let batch: Vec<u64> = queued.iter().copied().take(INBOX_BATCH_MAX).collect();
    let messages = batch
        .iter()
        .filter_map(|seq| {
            current
                .messages
                .iter()
                .find(|message| message.seq == *seq)
                .cloned()
        })
        .collect();
    Ok(InboxBatch {
        has_more: queued.len() > batch.len(),
        batch_id: batch.last().copied().unwrap_or(0),
        messages,
    })
}

/// Chat-kortets inkrementelle laesemodel. `from_seq` er eksklusiv.
pub fn view(thread: &str, from_seq: u64) -> Result<ThreadView, String> {
    let map = lock_threads();
    let current = map
        .get(thread)
        .ok_or_else(|| format!("unknown thread: {thread}"))?;
    Ok(ThreadView {
        state: current.state.as_str(),
        purpose: current.purpose.clone(),
        hops_used: current.hops_used,
        hops_left: MAX_HOPS.saturating_sub(current.hops_used),
        messages: current
            .messages
            .iter()
            .filter(|message| message.seq > from_seq)
            .map(|message| MessageView {
                seq: message.seq,
                from_card: message.from_card.clone(),
                from_kind: message.from_kind.as_str(),
                intent: message.intent.as_str(),
                text: message.text.clone(),
                ts_ms: message.ts_ms,
            })
            .collect(),
    })
}

/// (traad, antal beskeder). Hjerteslagets aendrings-detektion — billigere end
/// en notify-seam gennem kernen, og den kan ikke glemmes af en kodesti.
pub fn message_counts() -> Vec<(String, u64)> {
    lock_threads()
        .values()
        .map(|t| (t.id.clone(), t.messages.len() as u64))
        .collect()
}

/// `(inbox-laengde, staar kortet i wake-saettet?)` under ÉN laasning.
///
/// Findes fordi heartbeaten (250 ms) ellers svarede paa "venter kortet?" ved at
/// kalde `get()` — som `clone()`er HELE traaden, inkl. hver beskeds body (op
/// til 16 KiB × 20 hop). Mens en assignee arbejder, gentages den kopi fire
/// gange i sekundet for at laese én bool.
pub fn inbox_state(thread: &str, card: &str) -> (usize, bool) {
    let map = lock_threads();
    let Some(t) = map.get(thread) else {
        return (0, false);
    };
    (
        t.inbox.get(card).map(|v| v.len()).unwrap_or(0),
        t.wake.contains(card),
    )
}

pub fn wake_len(thread: &str) -> usize {
    lock_threads()
        .get(thread)
        .map(|t| t.wake.len())
        .unwrap_or(0)
}

pub fn wakes() -> Vec<(String, String)> {
    let map = lock_threads();
    let mut out = Vec::new();
    for t in map.values() {
        for card in &t.wake {
            out.push((card.clone(), t.id.clone()));
        }
    }
    out
}

pub fn clear_wake(card: &str, thread: &str) {
    if let Some(t) = lock_threads().get_mut(thread) {
        t.wake.remove(card);
    }
}

pub fn threads_with_member(card: &str) -> Vec<String> {
    lock_threads()
        .values()
        .filter(|t| t.state != ThreadState::Closed && t.members.iter().any(|m| m == card))
        .map(|t| t.id.clone())
        .collect()
}

#[cfg(feature = "test-seams")]
pub fn force_state_for_test(id: &str, state: ThreadState) {
    if let Some(t) = lock_threads().get_mut(id) {
        t.state = state;
    }
}

#[cfg(feature = "test-seams")]
pub fn force_hops_for_test(id: &str, hops: u32) {
    if let Some(t) = lock_threads().get_mut(id) {
        t.hops_used = hops;
    }
}
