//! `card_pair`-transaktionen (spec §3.5). Logikken bor i LIB'en mod injicerede
//! seams, fordi `spawn_into` bor i binary-craten og ikke kan naas fra `tests/`.
//!
//! Create-vejen er BEST-EFFORT: et fejlet spawn bevarer kortet og returnerer
//! Ok. Transaktionen skal derfor rydde eksplicit - en agent uden
//! Talminal-tools kan ikke kalde `card_inbox` og er ubrugelig som partner.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use super::{policy, FromKind, Intent, PostRequest, MAX_MESSAGE_BYTES};

pub const MAX_CHILDREN: u32 = 2;

#[derive(Debug)]
pub struct SpawnedCard {
    pub name: String,
    pub agent: String,
}

#[derive(Debug)]
pub struct PairResult {
    pub thread: String,
    pub partner: String,
    pub chat_card: String,
}

pub trait Spawner: Send + Sync + 'static {
    fn spawn(&self, agent: &str, cwd: &str) -> Result<SpawnedCard, String>;

    /// Skal fejle hvis kortet ikke koerer ELLER hvis MCP-configen ikke blev
    /// injiceret. En degraderet partner er ikke en partner.
    fn verify_running_with_mcp(&self, card: &str) -> Result<(), String>;

    fn close(&self, card: &str);
}

fn spawner() -> &'static Mutex<Option<Arc<dyn Spawner>>> {
    static S: OnceLock<Mutex<Option<Arc<dyn Spawner>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

pub fn set_spawner(s: Arc<dyn Spawner>) {
    *spawner().lock().unwrap_or_else(|p| p.into_inner()) = Some(s);
}

/// Kort der SELV er spawnet af en parring. Dybde 1: de maa ikke parre videre.
fn spawned_cards() -> &'static Mutex<HashSet<String>> {
    static S: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashSet::new()))
}

/// KUMULATIVT pr. app-session: kun dekrementeret naar transaktionen ruller
/// tilbage, saa et crashet eller lukket barn er brugt budget.
fn children() -> &'static Mutex<HashMap<String, u32>> {
    static C: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn thread_counter() -> &'static Mutex<u64> {
    static N: OnceLock<Mutex<u64>> = OnceLock::new();
    N.get_or_init(|| Mutex::new(0))
}

#[cfg(feature = "test-seams")]
pub fn reset_for_test() {
    spawned_cards()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear();
    children().lock().unwrap_or_else(|p| p.into_inner()).clear();
    *spawner().lock().unwrap_or_else(|p| p.into_inner()) = None;
    *thread_counter().lock().unwrap_or_else(|p| p.into_inner()) = 0;
}

fn next_thread_id() -> String {
    let mut n = thread_counter().lock().unwrap_or_else(|p| p.into_inner());
    *n += 1;
    format!("t{n}")
}

/// Loefter taelleren op over det hoejeste `t<N>.jsonl` der allerede ligger i
/// arkivet. Kaldes én gang ved opstart, foer den foerste parring.
///
/// Uden dette pas starter taelleren paa nul i hver proces, mens arkivet aldrig
/// slettes eller roteres (`append_lines` aabner med create+append): foerste
/// parring efter en genstart ville derfor faa "t1" igen og laegge sine linjer
/// bag en FREMMED samtales i samme fil. Id'et forbliver kort og talbart — det
/// staar i UI'et, i voice-svarene og i operatoer-runbooken, saa et UUID ville
/// koste mere end det giver; vi flytter kun startpunktet.
///
/// Robust med vilje: en mappe der ikke kan laeses, og filnavne der ikke er
/// traad-id'er, springes over — kun NAVNENE laeses, ikke indholdet. Taelleren
/// saenkes aldrig, saa et kald efter en parring ikke kan genudstede et id der
/// allerede er givet.
pub fn seed_counter_from_disk() {
    let dir = super::archive::threads_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut highest = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        if let Some(number) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(super::archive::thread_number)
        {
            highest = highest.max(number);
        }
    }
    let mut counter = thread_counter().lock().unwrap_or_else(|p| p.into_inner());
    *counter = (*counter).max(highest);
}

pub fn card_pair(
    from_card: &str,
    agent: &str,
    purpose: &str,
    opening: &str,
) -> Result<PairResult, String> {
    // Aabningsbeskeden valideres FOER noget reserveres eller oprettes. Trin 6
    // poster den gennem `post()`, som haandhaever samme loft — men dét kald
    // ligger efter commit-punktet, hvor der ikke laengere er en vej tilbage.
    // Uden denne vagt efterlader en for lang besked en koerende partner, et
    // synligt chat-kort og et BRUGT barne-slot bag en Err som kalderen ikke
    // har noget haandtag paa. Loftet er kumulativt, saa slottet er tabt for
    // resten af sessionen.
    if opening.len() > MAX_MESSAGE_BYTES {
        return Err(format!(
            "aabningsbeskeden er for lang ({} bytes, loft {MAX_MESSAGE_BYTES}) - opsummer",
            opening.len()
        ));
    }
    if spawned_cards()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(from_card)
    {
        return Err(format!(
            "depth limit: {from_card} was itself spawned by a pairing and may not pair again"
        ));
    }

    {
        let mut c = children().lock().unwrap_or_else(|p| p.into_inner());
        let used = c.entry(from_card.to_string()).or_insert(0);
        if *used >= MAX_CHILDREN {
            return Err(format!(
                "child limit: {from_card} has already used {MAX_CHILDREN} children this session"
            ));
        }
        *used += 1;
    }
    let release_slot = || {
        if let Some(used) = children()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(from_card)
        {
            *used = used.saturating_sub(1);
        }
    };

    let Some(sp) = spawner().lock().unwrap_or_else(|p| p.into_inner()).clone() else {
        release_slot();
        return Err("no spawner installed".to_string());
    };

    let cwd = crate::registry::card_cwd(from_card).unwrap_or_default();

    let partner = match sp.spawn(agent, &cwd) {
        Ok(partner) => partner,
        Err(error) => {
            release_slot();
            return Err(error);
        }
    };

    if let Err(error) = sp.verify_running_with_mcp(&partner.name) {
        sp.close(&partner.name);
        release_slot();
        return Err(format!(
            "partner unusable ({error}) - mcp capability missing"
        ));
    }

    let thread = next_thread_id();
    if let Err(error) = super::create_thread(
        &thread,
        purpose,
        vec![from_card.to_string(), partner.name.clone()],
    ) {
        sp.close(&partner.name);
        release_slot();
        return Err(error);
    }
    let chat_card =
        match crate::registry::create_chat_card(&thread, &super::sanitize_purpose(purpose)) {
            Ok(info) => info.name,
            Err(error) => {
                super::drop_thread_for_rollback(&thread);
                sp.close(&partner.name);
                release_slot();
                return Err(error);
            }
        };

    if let Err(error) = policy::pair(from_card, &partner.name) {
        // Samtykket skrives i BEGGE korts politik, ét kort ad gangen: fejler
        // den anden skrivning, staar peeren allerede i foraeldrenes liste. En
        // halvt anvendt parring er stadig en parring for `post()`, saa den
        // rulles tilbage her — `revoke` er tovejs og taaler at der intet var.
        policy::revoke(from_card, &partner.name);
        // Traaden droppes FOER chat-kortet lukkes. Naar T11's livscyklus-hook
        // lander, kalder `close_card` -> `on_card_gone`, som ville skrive et
        // terminalt udfald i arkivet for en traad der aldrig blev synlig —
        // praecis det `drop_thread_for_rollback` lover ikke sker. Samme
        // raekkefoelge som de oevrige rollback-veje: traaden vaek foerst.
        super::drop_thread_for_rollback(&thread);
        let _ = crate::registry::close_card(chat_card);
        sp.close(&partner.name);
        release_slot();
        return Err(error);
    }
    spawned_cards()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(partner.name.clone());

    super::post(PostRequest {
        thread: thread.clone(),
        from_card: from_card.to_string(),
        from_kind: FromKind::Agent,
        intent: Intent::Sparring,
        text: opening.to_string(),
    })?;

    // Transaktionen er i hus, og BEGGE kort findes. Fladen poller ikke
    // kortlisten, saa uden dette signal ville partneren og chat-kortet ligge
    // usynlige indtil ejeren tilfaeldigvis selv oprettede et kort (dogfood-fund
    // 2026-07-25). Kun her paa succes-vejen: enhver rollback ovenfor efterlader
    // kortlisten praecis som den var.
    crate::registry::notify_cards_changed();

    Ok(PairResult {
        thread,
        partner: partner.name,
        chat_card,
    })
}
