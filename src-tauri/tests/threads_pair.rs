mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use talminal_canvas_lib::threads::pair::{self, SpawnedCard, Spawner};
use talminal_canvas_lib::threads::policy::{AcceptsFromView, Port};
use talminal_canvas_lib::threads::{self, ThreadState};

#[derive(Default)]
struct FakeSpawner {
    spawned: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
    fail_spawn: Mutex<bool>,
    fail_verify: Mutex<bool>,
}

impl Spawner for FakeSpawner {
    fn spawn(&self, agent: &str, _cwd: &str) -> Result<SpawnedCard, String> {
        if *self.fail_spawn.lock().unwrap() {
            return Err("spawn failed".into());
        }
        let mut spawned = self.spawned.lock().unwrap();
        let name = format!("partner-{}", spawned.len() + 1);
        spawned.push(name.clone());
        Ok(SpawnedCard {
            name,
            agent: agent.into(),
        })
    }

    fn verify_running_with_mcp(&self, _card: &str) -> Result<(), String> {
        if *self.fail_verify.lock().unwrap() {
            return Err("mcp config missing".into());
        }
        Ok(())
    }

    fn close(&self, card: &str) {
        self.closed.lock().unwrap().push(card.into());
    }
}

fn setup() -> Arc<FakeSpawner> {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    pair::reset_for_test();
    let s = Arc::new(FakeSpawner::default());
    pair::set_spawner(s.clone());
    s
}

#[test]
fn the_happy_path_creates_partner_thread_chat_card_and_opening_message() {
    let _g = common::serial();
    let s = setup();
    let r = pair::card_pair("card-1", "codex", "spar om submit", "Jeg foreslaar ...").unwrap();
    assert_eq!(s.spawned.lock().unwrap().len(), 1);
    assert!(
        s.closed.lock().unwrap().is_empty(),
        "intet rives ned paa den lykkelige vej"
    );

    let t = threads::get(&r.thread).unwrap();
    assert_eq!(t.state, ThreadState::Open);
    assert_eq!(t.messages.len(), 1, "aabningsbeskeden er postet");
    assert!(t.members.contains(&"card-1".to_string()));
    assert!(t.members.contains(&r.partner));
    assert_eq!(threads::inbox_len(&r.thread, &r.partner), 1);
    assert!(t.wake.contains(&r.partner));

    let chat = talminal_canvas_lib::registry::chat_thread_id(&r.chat_card);
    assert_eq!(chat.as_deref(), Some(r.thread.as_str()));
    talminal_canvas_lib::registry::close_card(r.chat_card).ok();
}

#[test]
fn a_failed_spawn_leaves_nothing_behind_and_frees_the_slot() {
    let _g = common::serial();
    let s = setup();
    *s.fail_spawn.lock().unwrap() = true;
    assert!(pair::card_pair("card-1", "codex", "p", "hej").is_err());
    *s.fail_spawn.lock().unwrap() = false;
    for _ in 0..pair::MAX_CHILDREN {
        pair::card_pair("card-1", "codex", "p", "hej").expect("slot blev ikke frigivet");
    }
}

#[test]
fn a_degraded_partner_without_mcp_is_torn_down_not_kept() {
    let _g = common::serial();
    let s = setup();
    *s.fail_verify.lock().unwrap() = true;
    let err = pair::card_pair("card-1", "codex", "p", "hej").unwrap_err();
    assert!(err.contains("mcp"), "{err}");
    assert_eq!(s.closed.lock().unwrap().len(), 1, "partneren skal lukkes");
    assert!(
        threads::all_thread_ids_for_test().is_empty(),
        "ingen forladt traad"
    );
}

#[test]
fn a_spawned_card_may_not_pair_again() {
    let _g = common::serial();
    let s = setup();
    let r = pair::card_pair("card-1", "codex", "p", "hej").unwrap();
    let _ = s;
    let err = pair::card_pair(&r.partner, "claude", "p", "hej").unwrap_err();
    assert!(
        err.contains("depth"),
        "dybde 1: et spawnet kort maa ikke spawne videre: {err}"
    );
    talminal_canvas_lib::registry::close_card(r.chat_card).ok();
}

#[test]
fn the_child_cap_is_cumulative_so_a_closed_child_still_counts() {
    let _g = common::serial();
    let s = setup();
    let first = pair::card_pair("card-1", "codex", "p", "a").unwrap();
    let second = pair::card_pair("card-1", "codex", "p", "b").unwrap();
    s.close(&second.partner);
    let err = pair::card_pair("card-1", "codex", "p", "c").unwrap_err();
    assert!(
        err.contains("child"),
        "kumulativt loft: et lukket barn er brugt budget: {err}"
    );
    for card in [first.chat_card, second.chat_card] {
        talminal_canvas_lib::registry::close_card(card).ok();
    }
}

#[test]
fn purpose_is_sanitized_before_it_becomes_a_card_title() {
    let _g = common::serial();
    let _s = setup();
    let raw = format!("{}\u{0007}{}", "a".repeat(70), "b");
    let r = pair::card_pair("card-1", "codex", &raw, "hej").unwrap();
    let t = threads::get(&r.thread).unwrap();
    assert_eq!(t.purpose.chars().count(), threads::MAX_PURPOSE_CHARS);
    assert!(!t.purpose.contains('\u{0007}'));
    talminal_canvas_lib::registry::close_card(r.chat_card).ok();
}

/// Samtykke-porten som naegter. Rollback-vejen i trin 5 havde ellers ingen
/// daekning overhovedet.
struct RefusingPolicy;

impl Port for RefusingPolicy {
    fn read(&self, _card: &str) -> AcceptsFromView {
        AcceptsFromView::Any
    }
    fn pair(&self, _a: &str, _b: &str) -> Result<(), String> {
        Err("policy refused the pairing".to_string())
    }
    fn revoke(&self, _a: &str, _b: &str) {}
}

/// Review-fund: trin 6 postede aabningsbeskeden med et bart `?`. `post()`
/// afviser tekst over `MAX_MESSAGE_BYTES`, og `opening` er kaldervalgt — MCP
/// sender agentens tekst uredigeret videre. Uden en vagt foran transaktionen
/// efterlod dét en koerende partner, et synligt chat-kort og et brugt slot.
#[test]
fn an_oversized_opening_is_rejected_before_anything_is_created() {
    let _g = common::serial();
    let s = setup();
    let too_long = "x".repeat(threads::MAX_MESSAGE_BYTES + 1);
    let err = pair::card_pair("card-1", "codex", "p", &too_long).unwrap_err();
    assert!(
        err.contains("aabningsbeskeden er for lang"),
        "vagten skal afvise, ikke post() efter commit-punktet: {err}"
    );
    assert!(
        s.spawned.lock().unwrap().is_empty(),
        "intet maa spawnes for en besked der aldrig kan postes"
    );
    assert!(
        threads::all_thread_ids_for_test().is_empty(),
        "ingen forladt traad"
    );

    // Loftet er kumulativt: et braendt slot vindes aldrig tilbage i sessionen.
    let mut cards = Vec::new();
    for _ in 0..pair::MAX_CHILDREN {
        let r = pair::card_pair("card-1", "codex", "p", "hej").expect("slottet blev braendt");
        cards.push(r.chat_card);
    }
    for card in cards {
        talminal_canvas_lib::registry::close_card(card).ok();
    }
}

/// Review-fund: rollbacken lukkede chat-kortet FOER traaden blev droppet.
/// Naar T11's `on_card_gone` lander, skriver dét luk et terminalt udfald i
/// arkivet for en traad der aldrig blev synlig. Arkiv-assertionen herunder er
/// den der faanger en fremtidig ombytning — de oevrige daekker selve vejen.
#[test]
fn a_refused_consent_rolls_back_without_leaving_an_archive_trace() {
    let _g = common::serial();
    let _home = common::temp_home();
    let s = setup();
    talminal_canvas_lib::threads::policy::set_port(Arc::new(RefusingPolicy));

    let err = pair::card_pair("card-1", "codex", "p", "hej").unwrap_err();
    assert!(err.contains("refused"), "{err}");
    assert_eq!(
        s.closed.lock().unwrap().len(),
        1,
        "partneren skal rives ned naar samtykket ikke kan gives"
    );
    assert!(
        threads::all_thread_ids_for_test().is_empty(),
        "ingen forladt traad"
    );
    assert!(
        !talminal_canvas_lib::threads::archive::thread_path("t1")
            .expect("t1 er et traad-id")
            .exists(),
        "en traad der aldrig blev synlig maa ikke efterlade et arkivspor"
    );
}

/// Samtykke-porten skriver i BEGGE korts politik, ét kort ad gangen (se
/// `RegistryPolicyPort::pair`). Fejler den anden skrivning, staar peeren
/// allerede i foraeldrenes `accepts_from` — og en halvt anvendt parring er
/// stadig en parring for `post()`. Rollbacken skal derfor ogsaa kalde `revoke`.
struct HalfApplyingPolicy {
    revoked: Mutex<Vec<(String, String)>>,
}

impl Port for HalfApplyingPolicy {
    fn read(&self, _card: &str) -> AcceptsFromView {
        AcceptsFromView::Any
    }
    fn pair(&self, _a: &str, _b: &str) -> Result<(), String> {
        Err("card is not a terminal: partner-1".to_string())
    }
    fn revoke(&self, a: &str, b: &str) {
        self.revoked.lock().unwrap().push((a.into(), b.into()));
    }
}

#[test]
fn a_half_applied_consent_is_revoked_when_the_pairing_rolls_back() {
    let _g = common::serial();
    let _home = common::temp_home();
    let _s = setup();
    let policy = Arc::new(HalfApplyingPolicy {
        revoked: Mutex::new(Vec::new()),
    });
    talminal_canvas_lib::threads::policy::set_port(policy.clone());

    assert!(pair::card_pair("card-1", "codex", "p", "hej").is_err());
    assert_eq!(
        *policy.revoked.lock().unwrap(),
        vec![("card-1".to_string(), "partner-1".to_string())],
        "peeren blev efterladt i foraeldrenes politik"
    );
}

#[test]
fn thread_ids_resume_above_the_highest_archived_thread() {
    let _g = common::serial();
    let home = common::temp_home();
    let _s = setup();
    let dir = home.path().join("threads");
    std::fs::create_dir_all(&dir).unwrap();
    // Hverken den hoejeste fil, den ikke-navngivne eller den fremmede endelse
    // maa forvirre passet.
    for name in ["t3.jsonl", "t7.jsonl", "noter.jsonl", "t99.txt"] {
        std::fs::write(dir.join(name), "").unwrap();
    }

    threads::seed_counter_from_disk();

    let r = pair::card_pair("card-1", "codex", "p", "hej").unwrap();
    assert_eq!(
        r.thread, "t8",
        "en genstart maa ikke genudstede et id der allerede har en arkivfil"
    );
    talminal_canvas_lib::registry::close_card(r.chat_card).ok();
}

#[test]
fn an_empty_archive_still_starts_at_t1() {
    let _g = common::serial();
    let _home = common::temp_home();
    let _s = setup();
    threads::seed_counter_from_disk();
    let r = pair::card_pair("card-1", "codex", "p", "hej").unwrap();
    assert_eq!(r.thread, "t1", "en manglende threads-mappe er ikke en fejl");
    talminal_canvas_lib::registry::close_card(r.chat_card).ok();
}

/// Dogfood-fund 2026-07-25 (operatoer-roegtesten): parringen lykkedes, men
/// hverken partneren eller chat-kortet dukkede op paa canvas. De laa i
/// backenden hele tiden — fladen POLLER IKKE kortlisten, og den opdaterer den
/// kun paa mount og paa de to browser-kort-events. Kort som en AGENT skaber
/// havde ingen vej ind. De blev foerst synlige da ejeren tilfaeldigvis selv
/// oprettede et kort, hvis `create_card`-vej trigger et refresh.
#[test]
fn a_successful_pairing_tells_the_frontend_that_the_card_list_changed() {
    let _g = common::serial();
    let s = setup();
    let calls = Arc::new(AtomicUsize::new(0));
    let sink = calls.clone();
    talminal_canvas_lib::registry::set_cards_changed(Box::new(move || {
        sink.fetch_add(1, Ordering::SeqCst);
    }));

    let r = pair::card_pair("card-1", "codex", "p", "hej").unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "partner OG chat-kort er skabt i backenden; uden signalet er de usynlige"
    );
    let _ = s;
    talminal_canvas_lib::registry::close_card(r.chat_card).ok();
}

/// Modstykket: en rullet-tilbage parring efterlader kortlisten praecis som den
/// var, saa den maa ikke paastaa at noget aendrede sig.
#[test]
fn a_rolled_back_pairing_does_not_claim_the_card_list_changed() {
    let _g = common::serial();
    let s = setup();
    *s.fail_spawn.lock().unwrap() = true;
    let calls = Arc::new(AtomicUsize::new(0));
    let sink = calls.clone();
    talminal_canvas_lib::registry::set_cards_changed(Box::new(move || {
        sink.fetch_add(1, Ordering::SeqCst);
    }));

    assert!(pair::card_pair("card-1", "codex", "p", "hej").is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0, "intet blev skabt");
}

#[test]
fn without_a_spawner_pairing_is_a_named_error() {
    let _g = common::serial();
    threads::reset_for_test();
    pair::reset_for_test();
    let err = pair::card_pair("card-1", "codex", "p", "hej").unwrap_err();
    assert!(err.contains("spawner"), "{err}");
}
