mod common;

use talminal_canvas_lib::threads::policy::AcceptsFromView;
use talminal_canvas_lib::threads::{self, FromKind, Intent, PostRequest, TerminalReason};

fn thread_with_traffic() {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    threads::create_thread(
        "t1",
        "spar om submit",
        vec!["card-1".into(), "card-2".into()],
    )
    .unwrap();
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: "card-1".into(),
        from_kind: FromKind::Agent,
        intent: Intent::Sparring,
        text: "Jeg foreslaar".into(),
    })
    .unwrap();
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: threads::OWNER.into(),
        from_kind: FromKind::Human,
        intent: Intent::Sparring,
        text: "Hold jer til v1".into(),
    })
    .unwrap();
}

#[test]
fn view_carries_state_hops_and_wire_strings() {
    let _g = common::serial();
    thread_with_traffic();
    let view = threads::view("t1", 0).unwrap();
    assert_eq!(view.state, "open");
    assert_eq!(view.purpose, "spar om submit");
    assert_eq!(view.hops_used, 1);
    assert_eq!(view.hops_left, threads::MAX_HOPS - 1);
    assert_eq!(view.messages.len(), 2);
    assert_eq!(view.messages[0].from_kind, "agent");
    assert_eq!(view.messages[1].from_kind, "human");
    assert_eq!(view.messages[1].from_card, threads::OWNER);
}

#[test]
fn from_seq_returns_only_newer_messages() {
    let _g = common::serial();
    thread_with_traffic();
    let view = threads::view("t1", 1).unwrap();
    assert_eq!(view.messages.len(), 1);
    assert_eq!(view.messages[0].seq, 2);
}

#[test]
fn a_closed_thread_reports_closed_and_keeps_its_history() {
    let _g = common::serial();
    thread_with_traffic();
    threads::close_thread("t1", TerminalReason::OwnerStopped, None);
    let view = threads::view("t1", 0).unwrap();
    assert_eq!(view.state, "closed");
    assert_eq!(view.messages.len(), 3);
    assert_eq!(view.messages[2].from_kind, "system");
}

#[test]
fn view_serializes_to_snake_case_wire_fields() {
    let _g = common::serial();
    thread_with_traffic();
    let json = serde_json::to_value(threads::view("t1", 0).unwrap()).unwrap();
    assert!(json.get("hops_used").is_some());
    assert!(json["messages"][0].get("from_card").is_some());
}

#[test]
fn an_unknown_thread_is_an_error() {
    let _g = common::serial();
    threads::reset_for_test();
    assert!(threads::view("nope", 0).is_err());
}
