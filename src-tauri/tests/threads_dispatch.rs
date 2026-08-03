mod common;
use common::FakeClock;

use std::sync::{Arc, Mutex};
use talminal_canvas_lib::threads::dispatch::{self, TickOutcome};
use talminal_canvas_lib::threads::policy::AcceptsFromView;
use talminal_canvas_lib::threads::{self, FromKind, Intent, PostRequest, ThreadState};

#[derive(Default)]
struct FakeNotifier {
    writes: Mutex<Vec<(String, String)>>,
    fail: Mutex<bool>,
    busy: Mutex<bool>,
}

impl dispatch::Notifier for FakeNotifier {
    fn is_busy(&self, _card: &str) -> bool {
        *self.busy.lock().unwrap()
    }

    fn write_notice(&self, card: &str, text: &str) -> Result<(), String> {
        if *self.fail.lock().unwrap() {
            return Err("pty write failed".into());
        }
        self.writes.lock().unwrap().push((card.into(), text.into()));
        Ok(())
    }
}

fn setup() -> (Arc<FakeNotifier>, Arc<Mutex<u64>>) {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    dispatch::reset_for_test();
    let notifier = Arc::new(FakeNotifier::default());
    let clock = Arc::new(Mutex::new(1_000u64));
    dispatch::set_seams(Arc::new(FakeClock(clock.clone())), notifier.clone());
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    (notifier, clock)
}

fn post_from_1(intent: Intent) {
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: "card-1".into(),
        from_kind: FromKind::Agent,
        intent,
        text: "x".into(),
    })
    .unwrap();
}

#[test]
fn three_messages_produce_one_coalesced_notice() {
    let _g = common::serial();
    let (notifier, _) = setup();
    for _ in 0..3 {
        post_from_1(Intent::Sparring);
    }
    assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::Delivered);
    let writes = notifier.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].1.contains("3 nye beskeder"));
    assert!(writes[0].1.contains("card_inbox"));
    assert!(writes[0].1.contains("card_say"));
    assert!(writes[0].1.is_ascii());
    assert_eq!(threads::wake_len("t1"), 0);
}

#[test]
fn one_message_uses_singular() {
    let _g = common::serial();
    let (notifier, _) = setup();
    post_from_1(Intent::Sparring);
    dispatch::deliver("card-2", "t1");
    assert!(notifier.writes.lock().unwrap()[0].1.contains("1 ny besked"));
}

#[test]
fn busy_does_not_consume_a_wake_attempt() {
    let _g = common::serial();
    let (notifier, _) = setup();
    post_from_1(Intent::Sparring);
    *notifier.busy.lock().unwrap() = true;
    for _ in 0..5 {
        assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::Busy);
    }
    *notifier.busy.lock().unwrap() = false;
    assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::Delivered);
    assert_eq!(notifier.writes.lock().unwrap().len(), 1);
}

#[test]
fn a_successful_delivery_clears_the_attempt_counter() {
    let _g = common::serial();
    let (notifier, _) = setup();
    post_from_1(Intent::Sparring);
    *notifier.fail.lock().unwrap() = true;
    assert_eq!(
        dispatch::deliver("card-2", "t1"),
        TickOutcome::Failed { attempts: 1 }
    );
    *notifier.fail.lock().unwrap() = false;
    assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::Delivered);
    post_from_1(Intent::Sparring);
    *notifier.fail.lock().unwrap() = true;
    assert_eq!(
        dispatch::deliver("card-2", "t1"),
        TickOutcome::Failed { attempts: 1 }
    );
}

#[test]
fn two_failed_attempts_dead_letter_close_the_thread_and_spare_the_dead_card() {
    let _g = common::serial();
    let (notifier, _) = setup();
    post_from_1(Intent::Delegation);
    *notifier.fail.lock().unwrap() = true;
    assert_eq!(
        dispatch::deliver("card-2", "t1"),
        TickOutcome::Failed { attempts: 1 }
    );
    assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::DeadLettered);
    let thread = threads::get("t1").unwrap();
    assert!(thread.pending.is_none());
    assert_eq!(thread.state, ThreadState::Closed);
    assert!(thread.messages.last().unwrap().text.contains("2 forsoeg"));
    assert!(!thread.wake.contains("card-2"));
    assert!(thread.wake.contains("card-1"));
}

#[test]
fn a_dead_lettered_job_is_not_retried_forever() {
    let _g = common::serial();
    let (notifier, _) = setup();
    post_from_1(Intent::Sparring);
    *notifier.fail.lock().unwrap() = true;
    dispatch::deliver("card-2", "t1");
    dispatch::deliver("card-2", "t1");
    assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::Nothing);
}

#[test]
fn clocks_start_at_delivery_to_the_assignee() {
    let _g = common::serial();
    let (_, clock) = setup();
    post_from_1(Intent::Delegation);
    assert!(threads::get("t1")
        .unwrap()
        .pending
        .unwrap()
        .delivered_at_ms
        .is_none());
    *clock.lock().unwrap() = 50_000;
    assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::Delivered);
    let pending = threads::get("t1").unwrap().pending.unwrap();
    assert_eq!(pending.delivered_at_ms, Some(50_000));
    assert_eq!(pending.idle_deadline_ms, Some(50_000 + dispatch::IDLE_MS));
    assert_eq!(
        pending.absolute_deadline_ms,
        Some(50_000 + dispatch::ABSOLUTE_MS)
    );
}

#[test]
fn a_delivery_to_someone_else_does_not_start_the_clocks() {
    let _g = common::serial();
    let (_, clock) = setup();
    post_from_1(Intent::Delegation);
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: threads::OWNER.into(),
        from_kind: FromKind::Human,
        intent: Intent::Sparring,
        text: "husk v1".into(),
    })
    .unwrap();
    *clock.lock().unwrap() = 50_000;
    assert_eq!(dispatch::deliver("card-1", "t1"), TickOutcome::Delivered);
    assert!(threads::get("t1")
        .unwrap()
        .pending
        .unwrap()
        .delivered_at_ms
        .is_none());
}

#[test]
fn run_once_covers_every_waiting_card() {
    let _g = common::serial();
    let (notifier, _) = setup();
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: threads::OWNER.into(),
        from_kind: FromKind::Human,
        intent: Intent::Sparring,
        text: "til begge".into(),
    })
    .unwrap();
    let outcomes = dispatch::run_once();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes
        .iter()
        .all(|(_, _, outcome)| *outcome == TickOutcome::Delivered));
    let cards: Vec<String> = notifier
        .writes
        .lock()
        .unwrap()
        .iter()
        .map(|(card, _)| card.clone())
        .collect();
    assert!(cards.contains(&"card-1".to_string()));
    assert!(cards.contains(&"card-2".to_string()));
    assert!(dispatch::run_once().is_empty());
}

#[test]
fn without_seams_delivery_is_a_no_op_not_a_panic() {
    let _g = common::serial();
    setup();
    dispatch::reset_for_test();
    post_from_1(Intent::Sparring);
    assert_eq!(dispatch::deliver("card-2", "t1"), TickOutcome::Nothing);
}

// Deadline-trappen er konstant-aritmetik: haandhaevet paa kompile-tid her
// (og i sweep.rs' modul-invariant) i stedet for som runtime-test.
const _: () = assert!(dispatch::BACKSTOP_MS > dispatch::ABSOLUTE_MS);
const _: () = assert!(dispatch::ABSOLUTE_MS > dispatch::IDLE_MS);
