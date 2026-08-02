mod common;

use talminal_canvas_lib::threads::policy::AcceptsFromView;
use talminal_canvas_lib::threads::{
    self, FromKind, Intent, PostRequest, TerminalReason, ThreadState, OWNER, SYSTEM,
};

fn open_thread() {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
}

fn agent(from: &str, intent: Intent) -> PostRequest {
    PostRequest {
        thread: "t1".into(),
        from_card: from.into(),
        from_kind: FromKind::Agent,
        intent,
        text: "x".into(),
    }
}

fn owner(intent: Intent) -> PostRequest {
    PostRequest {
        thread: "t1".into(),
        from_card: OWNER.into(),
        from_kind: FromKind::Human,
        intent,
        text: "hold jer til v1".into(),
    }
}

#[test]
fn unknown_thread_is_error_not_panic() {
    let _g = common::serial();
    threads::reset_for_test();
    let mut req = agent("card-1", Intent::Sparring);
    req.thread = "nope".into();
    let err = threads::post(req).unwrap_err();
    assert!(err.contains("nope"), "{err}");
}

#[test]
fn non_member_agent_is_rejected() {
    let _g = common::serial();
    open_thread();
    let err = threads::post(agent("card-9", Intent::Sparring)).unwrap_err();
    assert!(err.contains("not a member"), "{err}");
}

#[test]
fn oversized_message_is_rejected_with_actionable_text() {
    let _g = common::serial();
    open_thread();
    let mut req = agent("card-1", Intent::Sparring);
    req.text = "a".repeat(threads::MAX_MESSAGE_BYTES + 1);
    let err = threads::post(req).unwrap_err();
    assert!(
        err.contains("opsummer"),
        "fejlen skal sige hvad afsenderen skal goere: {err}"
    );
}

#[test]
fn closed_thread_rejects_further_traffic() {
    let _g = common::serial();
    open_thread();
    threads::force_state_for_test("t1", ThreadState::Closed);
    let err = threads::post(agent("card-1", Intent::Sparring)).unwrap_err();
    assert!(err.contains("closed"), "{err}");
}

#[test]
fn delegation_is_rejected_while_awaiting_in_both_directions() {
    let _g = common::serial();
    open_thread();
    threads::post(agent("card-1", Intent::Delegation)).unwrap();
    assert_eq!(threads::get("t1").unwrap().state, ThreadState::Awaiting);
    let err = threads::post(agent("card-1", Intent::Delegation)).unwrap_err();
    assert!(err.contains("awaiting"), "samme retning: {err}");
    let err = threads::post(agent("card-2", Intent::Delegation)).unwrap_err();
    assert!(err.contains("awaiting"), "krydsdelegering: {err}");
}

#[test]
fn sparring_and_status_are_allowed_while_awaiting() {
    let _g = common::serial();
    open_thread();
    threads::post(agent("card-1", Intent::Delegation)).unwrap();
    threads::post(agent("card-2", Intent::Status)).unwrap();
    threads::post(agent("card-1", Intent::Sparring)).unwrap();
}

#[test]
fn only_an_agent_may_delegate() {
    let _g = common::serial();
    open_thread();
    let err = threads::post(owner(Intent::Delegation)).unwrap_err();
    assert!(err.contains("agent"), "{err}");
}

#[test]
fn the_owner_may_always_write_and_reaches_both_agents() {
    let _g = common::serial();
    open_thread();
    threads::policy::set_fixed_for_test(AcceptsFromView::Nobody);
    threads::post(owner(Intent::Sparring))
        .expect("ejeren er ikke medlem og skal alligevel kunne skrive");
    assert_eq!(threads::inbox_len("t1", "card-1"), 1);
    assert_eq!(threads::inbox_len("t1", "card-2"), 1);
    assert_eq!(threads::get("t1").unwrap().hops_used, 0);
}

#[test]
fn a_blocked_policy_stops_agents_but_not_the_owner() {
    let _g = common::serial();
    open_thread();
    threads::policy::set_fixed_for_test(AcceptsFromView::HumanOnly);
    let err = threads::post(agent("card-1", Intent::Sparring)).unwrap_err();
    assert!(err.contains("accepts_from"), "{err}");
    threads::post(owner(Intent::Sparring)).expect("human-only blokerer ikke mennesket");
}

#[test]
fn a_listed_peer_passes_the_policy() {
    let _g = common::serial();
    open_thread();
    threads::policy::set_fixed_for_test(AcceptsFromView::List(vec!["card-1".into()]));
    threads::post(agent("card-1", Intent::Sparring)).expect("parret kort maa skrive");
    let err = threads::post(agent("card-2", Intent::Sparring)).unwrap_err();
    assert!(err.contains("accepts_from"), "kun det listede kort: {err}");
}

#[test]
fn agent_messages_increment_monotonically() {
    let _g = common::serial();
    open_thread();
    let a = threads::post(agent("card-1", Intent::Sparring)).unwrap();
    let b = threads::post(agent("card-2", Intent::Sparring)).unwrap();
    assert_eq!((a.hop, b.hop), (1, 2));
    assert_eq!(b.hops_left, threads::MAX_HOPS - 2);
}

#[test]
fn message_20_is_accepted_and_21_closes_the_thread_with_a_trace() {
    let _g = common::serial();
    open_thread();
    for i in 1..=20 {
        let from = if i % 2 == 1 { "card-1" } else { "card-2" };
        let accepted = threads::post(agent(from, Intent::Sparring))
            .unwrap_or_else(|e| panic!("besked {i} skulle passere: {e}"));
        assert_eq!(accepted.hop, i);
    }
    let err = threads::post(agent("card-1", Intent::Sparring)).unwrap_err();
    assert!(err.contains("hop"), "{err}");
    let t = threads::get("t1").unwrap();
    assert_eq!(t.state, ThreadState::Closed);
    let last = t.messages.last().unwrap();
    assert_eq!(last.from_kind, FromKind::System);
    assert!(last.text.contains("20"));
    assert_eq!(threads::wake_len("t1"), 2);
}

#[test]
fn an_answer_on_the_pending_delegation_always_passes_the_cap() {
    let _g = common::serial();
    open_thread();
    threads::post(agent("card-1", Intent::Delegation)).unwrap();
    threads::force_hops_for_test("t1", 20);
    threads::post(agent("card-2", Intent::Answer)).expect("answer passerer loftet");
    let t = threads::get("t1").unwrap();
    assert!(t.pending.is_none());
    assert_eq!(t.state, ThreadState::Open);
}

/// Kontrasten der gør beslutning 11 til noget: ved loftet passerer et `answer`,
/// men alt ANDET afvises - og afvisningen tager den udestaaende delegering med
/// sig gennem den faelles terminalvej. Planens oprindelige test forsoegte at
/// paastaa begge halvdele i ét forloeb og var derfor selvmodsigende (sparringen
/// lukker traaden, saa svaret bagefter ramte "thread is closed"). To tests i
/// stedet for én: dette er den anden halvdel, og det er den vej rev 1.0 tabte
/// udfaldet paa.
#[test]
fn sparring_at_the_cap_is_rejected_and_takes_the_pending_delegation_with_it() {
    let _g = common::serial();
    open_thread();
    threads::post(agent("card-1", Intent::Delegation)).unwrap();
    threads::force_hops_for_test("t1", 20);

    let err = threads::post(agent("card-2", Intent::Sparring)).unwrap_err();
    assert!(err.contains("hop"), "{err}");

    let t = threads::get("t1").unwrap();
    assert_eq!(t.state, ThreadState::Closed);
    assert!(
        t.pending.is_none(),
        "hop-loftet skal ogsaa afslutte den udestaaende delegering"
    );
    assert!(t.messages.last().unwrap().text.contains("20"));
    assert_eq!(
        threads::wake_len("t1"),
        2,
        "begge agenter skal vaekkes af udfaldet"
    );
    assert_eq!(
        threads::inbox_len("t1", "card-1"),
        1,
        "rekvirenten skal se udfaldet"
    );
}

#[test]
fn an_answer_from_the_wrong_card_does_not_close_the_delegation() {
    let _g = common::serial();
    open_thread();
    threads::post(agent("card-1", Intent::Delegation)).unwrap();
    threads::post(agent("card-1", Intent::Answer)).unwrap();
    assert!(threads::get("t1").unwrap().pending.is_some());
}

#[test]
fn an_answer_writes_no_system_message() {
    let _g = common::serial();
    open_thread();
    threads::post(agent("card-1", Intent::Delegation)).unwrap();
    threads::post(agent("card-2", Intent::Answer)).unwrap();
    let msgs = threads::get("t1").unwrap().messages;
    assert!(msgs.iter().all(|m| m.from_kind != FromKind::System));
}

#[test]
fn closing_writes_exactly_one_system_message_and_is_idempotent() {
    let _g = common::serial();
    open_thread();
    threads::post(agent("card-1", Intent::Delegation)).unwrap();
    assert!(threads::close_thread(
        "t1",
        TerminalReason::OwnerStopped,
        None
    ));
    assert!(!threads::close_thread(
        "t1",
        TerminalReason::OwnerStopped,
        None
    ));
    let t = threads::get("t1").unwrap();
    assert_eq!(t.state, ThreadState::Closed);
    assert!(t.pending.is_none());
    assert_eq!(
        t.messages
            .iter()
            .filter(|m| m.from_kind == FromKind::System)
            .count(),
        1
    );
    assert_eq!(t.messages.last().unwrap().from_card, SYSTEM);
}

#[test]
fn closing_a_thread_without_a_pending_still_writes_the_outcome() {
    let _g = common::serial();
    open_thread();
    threads::post(agent("card-1", Intent::Sparring)).unwrap();
    assert!(threads::close_thread(
        "t1",
        TerminalReason::OwnerStopped,
        None
    ));
    assert!(threads::get("t1")
        .unwrap()
        .messages
        .last()
        .unwrap()
        .text
        .contains("stoppede"));
}

#[test]
fn every_terminal_path_clears_stale_state_and_then_notifies_the_survivor() {
    for reason in [
        TerminalReason::OwnerStopped,
        TerminalReason::ParticipantLost,
        TerminalReason::HopLimit,
        TerminalReason::RestartAbort,
        TerminalReason::DeliveryFailed,
    ] {
        let _g = common::serial();
        open_thread();
        threads::post(agent("card-1", Intent::Delegation)).unwrap();
        threads::close_thread("t1", reason, Some("card-2"));
        let t = threads::get("t1").unwrap();
        assert_eq!(t.state, ThreadState::Closed);
        assert!(t.pending.is_none());
        assert_eq!(threads::inbox_len("t1", "card-2"), 0);
        assert!(!t.wake.contains("card-2"));
        assert_eq!(threads::inbox_len("t1", "card-1"), 1);
        assert!(t.wake.contains("card-1"));
    }
}

#[test]
fn terminal_text_is_derived_from_reason_and_peer_active() {
    let idle = threads::terminal_text(TerminalReason::IdleTimeout { peer_active: false }, "card-2");
    let hard_active = threads::terminal_text(
        TerminalReason::AbsoluteTimeout { peer_active: true },
        "card-2",
    );
    let hard_idle = threads::terminal_text(
        TerminalReason::AbsoluteTimeout { peer_active: false },
        "card-2",
    );
    assert!(idle.contains('5'));
    assert!(hard_active.contains("arbejdede"));
    assert_ne!(hard_active, hard_idle);
    assert_ne!(idle, hard_idle);
}

#[test]
fn all_nine_reasons_have_distinct_non_empty_text() {
    let reasons = [
        TerminalReason::Answer,
        TerminalReason::IdleTimeout { peer_active: false },
        TerminalReason::AbsoluteTimeout { peer_active: false },
        TerminalReason::BackstopCleanup,
        TerminalReason::DeliveryFailed,
        TerminalReason::ParticipantLost,
        TerminalReason::OwnerStopped,
        TerminalReason::RestartAbort,
        TerminalReason::HopLimit,
    ];
    let mut seen = std::collections::HashSet::new();
    for r in reasons {
        let t = threads::terminal_text(r, "card-2");
        assert!(!t.trim().is_empty());
        assert!(seen.insert(t));
    }
}

#[test]
fn each_accepted_message_becomes_one_json_line() {
    let _g = common::serial();
    let _home = common::temp_home();
    open_thread();
    let mut req = agent("card-1", Intent::Sparring);
    req.text = "linje\nmed\nlinjeskift".into();
    threads::post(req).unwrap();
    threads::close_thread("t1", TerminalReason::OwnerStopped, None);
    let body =
        std::fs::read_to_string(threads::archive::thread_path("t1").expect("t1 er et traad-id"))
            .unwrap();
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2);
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["from_kind"], "agent");
    assert!(first["text"].as_str().unwrap().contains('\n'));
    assert!(first.get("terminal_reason").is_none());
    let last: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(last["from_kind"], "system");
    assert_eq!(last["terminal_reason"], "owner_stopped");
}
