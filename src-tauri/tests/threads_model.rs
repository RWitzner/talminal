mod common;

use talminal_canvas_lib::threads::{self, FromKind, Intent, ThreadState};

#[test]
fn create_thread_starts_open_with_zero_hops() {
    let _g = common::serial();
    threads::reset_for_test();
    threads::create_thread(
        "t1",
        "spar om submit",
        vec!["card-1".into(), "card-2".into()],
    )
    .unwrap();
    let t = threads::get("t1").expect("thread findes");
    assert_eq!(t.state, ThreadState::Open);
    assert_eq!(t.hops_used, 0);
    assert!(t.pending.is_none());
    assert!(t.inbox.is_empty(), "ingen har noget ukvitteret endnu");
    assert!(t.wake.is_empty(), "ingen wake foer foerste besked");
    assert_eq!(t.members, vec!["card-1".to_string(), "card-2".to_string()]);
}

#[test]
fn purpose_is_truncated_to_60_chars_and_control_chars_stripped() {
    let _g = common::serial();
    threads::reset_for_test();
    let raw = format!("{}\u{0007}\n{}", "a".repeat(58), "b".repeat(30));
    threads::create_thread("t2", &raw, vec!["card-1".into(), "card-2".into()]).unwrap();
    let t = threads::get("t2").unwrap();
    assert_eq!(t.purpose.chars().count(), 60);
    assert!(!t.purpose.contains('\u{0007}'));
    assert!(!t.purpose.contains('\n'));
}

#[test]
fn duplicate_thread_id_is_rejected() {
    let _g = common::serial();
    threads::reset_for_test();
    threads::create_thread("t3", "x", vec!["card-1".into(), "card-2".into()]).unwrap();
    let err =
        threads::create_thread("t3", "y", vec!["card-1".into(), "card-2".into()]).unwrap_err();
    assert!(err.contains("t3"), "fejlen skal navngive traaden: {err}");
}

#[test]
fn a_thread_needs_exactly_two_distinct_agent_members() {
    let _g = common::serial();
    threads::reset_for_test();
    let err = threads::create_thread("t4", "x", vec!["card-1".into()]).unwrap_err();
    assert!(err.contains("two"), "{err}");
    let err =
        threads::create_thread("t5", "x", vec!["card-1".into(), "card-1".into()]).unwrap_err();
    assert!(err.contains("distinct"), "{err}");
    let err = threads::create_thread("t6", "x", vec!["card-1".into(), "owner".into()]).unwrap_err();
    assert!(err.contains("reserved"), "{err}");
}

#[test]
fn wire_strings_are_stable() {
    assert_eq!(FromKind::Agent.as_str(), "agent");
    assert_eq!(FromKind::Human.as_str(), "human");
    assert_eq!(FromKind::System.as_str(), "system");
    assert_eq!(Intent::Sparring.as_str(), "sparring");
    assert_eq!(Intent::Delegation.as_str(), "delegation");
    assert_eq!(Intent::Answer.as_str(), "answer");
    assert_eq!(Intent::Status.as_str(), "status");
}

#[test]
fn an_unwired_policy_port_denies_agents_and_says_so() {
    let _g = common::serial();
    threads::policy::reset_for_test();
    assert!(!threads::policy::is_wired(), "frisk proces har ingen port");
    assert!(!threads::policy::read("card-2").allows("card-1"));
}
