mod common;

use std::path::PathBuf;
use std::sync::Arc;
use talminal_canvas_lib::cards::CardConfig;
use talminal_canvas_lib::registry::{self, AcceptsFrom};
use talminal_canvas_lib::threads::policy;

fn seed(name: &str) {
    let cfg = CardConfig {
        name: name.to_string(),
        cwd: PathBuf::from("."),
        command: vec!["claude".into()],
        resume_command: vec!["claude".into(), "--continue".into()],
    };
    registry::seed_card(cfg, "claude").expect("seed_card");
}

#[test]
fn default_policy_is_human_only() {
    assert_eq!(AcceptsFrom::default(), AcceptsFrom::HumanOnly);
}

#[test]
fn a_fresh_card_is_not_agent_writable() {
    let _g = common::serial();
    policy::set_port(Arc::new(registry::RegistryPolicyPort));
    seed("card-p1");
    assert!(!policy::read("card-p1").allows("card-p2"));
}

#[test]
fn pair_makes_exactly_the_two_cards_mutually_writable() {
    let _g = common::serial();
    policy::set_port(Arc::new(registry::RegistryPolicyPort));
    seed("card-p3");
    seed("card-p4");
    seed("card-p5");
    policy::pair("card-p3", "card-p4").expect("pair");
    assert!(policy::read("card-p3").allows("card-p4"));
    assert!(policy::read("card-p4").allows("card-p3"));
    assert!(!policy::read("card-p3").allows("card-p5"));
}

#[test]
fn revoke_removes_only_the_pair_entry() {
    let _g = common::serial();
    policy::set_port(Arc::new(registry::RegistryPolicyPort));
    seed("card-p6");
    seed("card-p7");
    policy::pair("card-p6", "card-p7").unwrap();
    policy::revoke("card-p6", "card-p7");
    assert!(!policy::read("card-p6").allows("card-p7"));
    assert!(!policy::read("card-p7").allows("card-p6"));
}

#[test]
fn an_unknown_card_reads_as_nobody_not_as_any() {
    let _g = common::serial();
    policy::set_port(Arc::new(registry::RegistryPolicyPort));
    assert_eq!(
        policy::read("card-does-not-exist"),
        policy::AcceptsFromView::Nobody
    );
}

#[test]
fn pairing_a_non_terminal_card_is_a_named_error() {
    let _g = common::serial();
    policy::set_port(Arc::new(registry::RegistryPolicyPort));
    seed("card-p8");
    let err = policy::pair("card-p8", "card-nope").unwrap_err();
    assert!(err.contains("card-nope"), "{err}");
}
