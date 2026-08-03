mod common;
use common::FakeClock;

use std::sync::{Arc, Mutex};
use talminal_canvas_lib::threads::dispatch::{self, TickOutcome};
use talminal_canvas_lib::threads::policy::AcceptsFromView;
use talminal_canvas_lib::threads::{self, sweep, FromKind, Intent, PostRequest, ThreadState};

#[derive(Default)]
struct FakeNotifier {
    busy: Mutex<bool>,
}

impl dispatch::Notifier for FakeNotifier {
    fn is_busy(&self, _card: &str) -> bool {
        *self.busy.lock().unwrap()
    }

    fn write_notice(&self, _card: &str, _text: &str) -> Result<(), String> {
        Ok(())
    }
}

fn setup() -> (Arc<Mutex<u64>>, Arc<FakeNotifier>) {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    threads::reset_activity_probe_for_test();
    dispatch::reset_for_test();
    sweep::reset_pre_close_hook_for_test();
    let clock = Arc::new(Mutex::new(1_000u64));
    let notifier = Arc::new(FakeNotifier::default());
    dispatch::set_seams(Arc::new(FakeClock(clock.clone())), notifier.clone());
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: "card-1".into(),
        from_kind: FromKind::Agent,
        intent: Intent::Delegation,
        text: "lav X".into(),
    })
    .unwrap();
    // `request_ts_ms` skrives af T2-kernens systemur. Fake-uret synkroniseres
    // til samme domaene uden at aendre den laaste kerne.
    *clock.lock().unwrap() = threads::get("t1").unwrap().pending.unwrap().request_ts_ms;
    (clock, notifier)
}

#[test]
fn the_idle_clock_fires_when_the_peer_is_quiet() {
    let _g = common::serial();
    let (clock, _) = setup();
    assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::Delivered);
    threads::set_activity_probe_for_test(false);
    *clock.lock().unwrap() += dispatch::IDLE_MS + 1;
    sweep::sweep();
    let thread = threads::get("t1").unwrap();
    assert!(thread.pending.is_none());
    assert!(thread.messages.last().unwrap().text.contains("5 minutter"));
}

#[test]
fn an_active_peer_survives_the_idle_clock_and_hits_the_hard_cap() {
    let _g = common::serial();
    let (clock, _) = setup();
    dispatch::deliver("card-2", "t1");
    threads::set_activity_probe_for_test(true);
    *clock.lock().unwrap() += dispatch::IDLE_MS + 1;
    sweep::sweep();
    assert!(threads::get("t1").unwrap().pending.is_some());

    *clock.lock().unwrap() += dispatch::ABSOLUTE_MS;
    sweep::sweep();
    let thread = threads::get("t1").unwrap();
    assert!(thread.pending.is_none());
    assert!(thread.messages.last().unwrap().text.contains("arbejdede"));
}

#[test]
fn the_backstop_fires_on_a_delegation_that_was_never_delivered() {
    let _g = common::serial();
    let (clock, notifier) = setup();
    *notifier.busy.lock().unwrap() = true;
    assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::Busy);
    assert!(threads::get("t1")
        .unwrap()
        .pending
        .unwrap()
        .absolute_deadline_ms
        .is_none());

    *clock.lock().unwrap() += dispatch::ABSOLUTE_MS + 1;
    sweep::sweep();
    assert!(threads::get("t1").unwrap().pending.is_some());

    *clock.lock().unwrap() += 60_000 + 1;
    sweep::sweep();
    let thread = threads::get("t1").unwrap();
    assert!(thread.pending.is_none());
    assert_eq!(thread.state, ThreadState::Closed);
    assert!(thread.messages.last().unwrap().text.contains("backstoppen"));
}

#[test]
fn a_delivered_delegation_is_never_taken_by_the_backstop_first() {
    let _g = common::serial();
    let (clock, _) = setup();
    *clock.lock().unwrap() += 10 * 60 * 1_000;
    dispatch::deliver("card-2", "t1");
    threads::set_activity_probe_for_test(false);
    *clock.lock().unwrap() += dispatch::ABSOLUTE_MS + 1;
    sweep::sweep();
    let text = threads::get("t1")
        .unwrap()
        .messages
        .last()
        .unwrap()
        .text
        .clone();
    assert!(text.contains("20 minutter"), "{text}");
}

#[test]
fn sweeping_is_idempotent_and_leaves_healthy_threads_alone() {
    let _g = common::serial();
    setup();
    dispatch::deliver("card-2", "t1");
    sweep::sweep();
    sweep::sweep();
    assert!(threads::get("t1").unwrap().pending.is_some());
}

/// Svaret lander PRAECIS i vinduet mellem sweeperens snapshot og dens lukning.
///
/// Vinduet er reelt: snapshottet bygges under traadlaasen, laasen slippes, og
/// derefter kaldes baade `peer_active` (tre andre laase) og lukningen. Uden
/// revalidering lukkede sweeperen paa en delegering der allerede var besvaret —
/// og `close_thread` rydder `inbox`/`wake`, saa svaret var i huset, men aldrig
/// blev leveret, og ejeren fik at vide at modparten ikke naaede at svare.
/// Hooken bruges frem for en `Barrier`, fordi den maaling allerede er gjort her
/// i repoet: lukkeren vandt 149 ud af 150 omgange.
#[test]
fn an_answer_that_lands_in_the_sweep_window_is_not_lost_to_the_timeout() {
    let _g = common::serial();
    let (clock, _) = setup();
    dispatch::deliver("card-2", "t1");
    threads::set_activity_probe_for_test(false);
    *clock.lock().unwrap() += dispatch::IDLE_MS + 1;

    sweep::set_pre_close_hook_for_test(Box::new(|| {
        threads::post(PostRequest {
            thread: "t1".into(),
            from_card: "card-2".into(),
            from_kind: FromKind::Agent,
            intent: Intent::Answer,
            text: "faerdig".into(),
        })
        .expect("svaret skal accepteres — delegeringen er stadig aaben");
    }));
    sweep::sweep();

    let thread = threads::get("t1").unwrap();
    assert_eq!(
        thread.state,
        ThreadState::Open,
        "svaret afsluttede delegeringen; timeouten maa ikke lukke traaden bagefter"
    );
    assert!(
        thread
            .messages
            .iter()
            .all(|m| m.from_kind != FromKind::System),
        "der blev skrevet et terminalt udfald for en delegering der var besvaret"
    );
    // Laengden alene skelner ikke: den GAMLE adfaerd ryddede koen og lagde sin
    // egen terminal-besked i den, saa card-1 havde ogsaa praecis én staaende.
    // Det er INDHOLDET der afgoer om leveringen overlevede.
    let inbox = threads::inbox_take("t1", "card-1", None).expect("card-1 er medlem af t1");
    let standing = inbox
        .messages
        .first()
        .expect("svaret skal stadig ligge i requesterens koe — det er selve leveringen");
    assert_eq!(
        standing.intent,
        Intent::Answer,
        "koen baerer et terminalt udfald i stedet for svaret: lukningen vandt kaploebet"
    );
    assert_eq!(
        standing.from_card, "card-2",
        "og det skal vaere modpartens svar, ikke systemets besked"
    );
    assert!(
        thread.wake.contains("card-1"),
        "og requesteren skal stadig vaekkes"
    );
}

/// Skaerpelsen: svaret bliver fulgt af en NY delegering i samme vindue, saa
/// traaden staar `Awaiting` igen naar sweeperen naar frem. En vagt der kun saa
/// paa tilstanden ville lukke — men det er en anden delegering med sine egne
/// frister, og den har lige faaet dem.
#[test]
fn a_fresh_delegation_in_the_sweep_window_keeps_its_own_deadlines() {
    let _g = common::serial();
    let (clock, _) = setup();
    dispatch::deliver("card-2", "t1");
    threads::set_activity_probe_for_test(false);
    *clock.lock().unwrap() += dispatch::IDLE_MS + 1;

    sweep::set_pre_close_hook_for_test(Box::new(|| {
        for (from, intent) in [("card-2", Intent::Answer), ("card-2", Intent::Delegation)] {
            threads::post(PostRequest {
                thread: "t1".into(),
                from_card: from.into(),
                from_kind: FromKind::Agent,
                intent,
                text: "x".into(),
            })
            .expect("baade svar og ny delegering skal passere");
        }
    }));
    sweep::sweep();

    let thread = threads::get("t1").unwrap();
    assert_eq!(thread.state, ThreadState::Awaiting);
    let pending = thread
        .pending
        .expect("den nye delegering staar stadig aaben");
    assert_eq!(
        pending.requester, "card-2",
        "det er den NYE delegering der er pending"
    );
    assert!(
        thread
            .messages
            .iter()
            .all(|m| m.from_kind != FromKind::System),
        "den gamle delegerings frist lukkede den nye"
    );
}

/// Modstykket: hooken maa ikke goere sweeperen tandloes. Sker der intet i
/// vinduet, lukker fristen som foer.
#[test]
fn an_untouched_delegation_still_times_out_through_the_window() {
    let _g = common::serial();
    let (clock, _) = setup();
    dispatch::deliver("card-2", "t1");
    threads::set_activity_probe_for_test(false);
    *clock.lock().unwrap() += dispatch::IDLE_MS + 1;

    sweep::set_pre_close_hook_for_test(Box::new(|| {}));
    sweep::sweep();

    let thread = threads::get("t1").unwrap();
    assert!(thread.pending.is_none());
    assert_eq!(thread.state, ThreadState::Closed);
}

#[test]
fn startup_invariants_hold() {
    let _g = common::serial();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    sweep::assert_startup_invariants();
}

#[test]
#[should_panic(expected = "policy port")]
fn startup_invariants_catch_an_unwired_policy_port() {
    let _g = common::serial();
    threads::policy::reset_for_test();
    sweep::assert_startup_invariants();
}
