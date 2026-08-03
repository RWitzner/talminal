mod common;
use common::FakeClock;

use std::sync::{Arc, Mutex};
use talminal_canvas_lib::threads::dispatch;
use talminal_canvas_lib::threads::heartbeat::{self, ChangeTracker};
use talminal_canvas_lib::threads::policy::AcceptsFromView;
use talminal_canvas_lib::threads::{self, FromKind, Intent, PostRequest, TerminalReason};

struct Silent;
impl dispatch::Notifier for Silent {
    fn is_busy(&self, _card: &str) -> bool {
        false
    }
    fn write_notice(&self, _card: &str, _text: &str) -> Result<(), String> {
        Ok(())
    }
}

fn setup() -> Arc<Mutex<u64>> {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    threads::reset_activity_probe_for_test();
    dispatch::reset_for_test();
    let clock = Arc::new(Mutex::new(1_000u64));
    dispatch::set_seams(Arc::new(FakeClock(clock.clone())), Arc::new(Silent));
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    clock
}

fn post_one() {
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: "card-1".into(),
        from_kind: FromKind::Agent,
        intent: Intent::Sparring,
        text: "x".into(),
    })
    .unwrap();
}

#[test]
fn a_beat_delivers_and_reports_the_changed_thread_once() {
    let _g = common::serial();
    setup();
    let mut tracker = ChangeTracker::default();
    post_one();
    let beat = heartbeat::beat(&mut tracker);
    assert_eq!(beat.changed, vec!["t1".to_string()]);
    assert_eq!(beat.delivered, 1);
    // Uden nye beskeder er slaget tomt: eventet maa ikke spamme fladen.
    let beat = heartbeat::beat(&mut tracker);
    assert!(beat.changed.is_empty());
    assert_eq!(beat.delivered, 0);
}

#[test]
fn a_terminal_message_also_counts_as_a_change() {
    let _g = common::serial();
    setup();
    let mut tracker = ChangeTracker::default();
    heartbeat::beat(&mut tracker);
    threads::close_thread("t1", TerminalReason::OwnerStopped, None);
    assert_eq!(
        heartbeat::beat(&mut tracker).changed,
        vec!["t1".to_string()]
    );
}

#[test]
fn the_sweeper_runs_on_every_nth_beat_only() {
    let _g = common::serial();
    let clock = setup();
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: "card-1".into(),
        from_kind: FromKind::Agent,
        intent: Intent::Delegation,
        text: "lav X".into(),
    })
    .unwrap();
    let mut tracker = ChangeTracker::default();
    heartbeat::beat(&mut tracker); // levering saetter urene
    threads::set_activity_probe_for_test(false);
    *clock.lock().unwrap() += dispatch::IDLE_MS + 1;

    for _ in 1..heartbeat::SWEEP_EVERY {
        heartbeat::beat(&mut tracker);
    }
    assert!(
        threads::get("t1").unwrap().pending.is_some(),
        "sweeperen koerer ikke paa hvert slag - deadlines er minutter, ikke millisekunder"
    );
    heartbeat::beat(&mut tracker);
    assert!(
        threads::get("t1").unwrap().pending.is_none(),
        "og den koerer paa det N'te"
    );
}

#[test]
fn a_closed_thread_stops_producing_changes() {
    let _g = common::serial();
    setup();
    let mut tracker = ChangeTracker::default();
    threads::close_thread("t1", TerminalReason::OwnerStopped, None);
    heartbeat::beat(&mut tracker);
    assert!(heartbeat::beat(&mut tracker).changed.is_empty());
}
