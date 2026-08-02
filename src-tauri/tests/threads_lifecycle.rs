mod common;

use talminal_canvas_lib::registry;
use talminal_canvas_lib::threads::policy::AcceptsFromView;
use talminal_canvas_lib::threads::{self, FromKind, Intent, PostRequest, ThreadState};

fn awaiting_thread(id: &str) {
    threads::create_thread(id, "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    threads::post(PostRequest {
        thread: id.into(),
        from_card: "card-1".into(),
        from_kind: FromKind::Agent,
        intent: Intent::Delegation,
        text: "lav X".into(),
    })
    .unwrap();
}

fn fresh() {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
}

#[test]
fn a_lost_participant_closes_the_thread_as_participant_lost() {
    let _g = common::serial();
    fresh();
    awaiting_thread("t1");
    assert_eq!(
        threads::on_card_gone("card-2", None),
        1,
        "én traad blev lukket"
    );
    let t = threads::get("t1").unwrap();
    assert_eq!(t.state, ThreadState::Closed);
    assert!(
        t.pending.is_none(),
        "delegeringen skal faa et terminalt udfald STRAKS"
    );
    assert!(
        t.messages.last().unwrap().text.contains("vaek"),
        "udfaldet skal vaere participant_lost, ikke en timeout: {}",
        t.messages.last().unwrap().text
    );
    // Det doede kort maa ikke vaekkes, men modparten skal have udfaldet.
    assert!(!t.wake.contains("card-2"));
    assert!(t.wake.contains("card-1"));
}

#[test]
fn closing_the_chat_card_closes_the_thread_as_owner_stopped() {
    let _g = common::serial();
    fresh();
    awaiting_thread("t1");
    assert_eq!(threads::on_card_gone("card-3", Some("t1")), 1);
    let t = threads::get("t1").unwrap();
    assert_eq!(t.state, ThreadState::Closed);
    assert!(
        t.messages.last().unwrap().text.contains("stoppede"),
        "lukning af chat-kortet er owner_stopped: {}",
        t.messages.last().unwrap().text
    );
    // Ejerens luk rammer ingen af agenterne som "tabt": begge skal vide det.
    assert!(t.wake.contains("card-1") && t.wake.contains("card-2"));
}

#[test]
fn a_card_in_several_threads_closes_all_of_them() {
    let _g = common::serial();
    fresh();
    awaiting_thread("t1");
    threads::create_thread("t2", "p", vec!["card-2".into(), "card-5".into()]).unwrap();
    assert_eq!(threads::on_card_gone("card-2", None), 2);
    assert_eq!(threads::get("t2").unwrap().state, ThreadState::Closed);
}

#[test]
fn a_non_member_card_leaves_the_thread_open() {
    let _g = common::serial();
    fresh();
    awaiting_thread("t1");
    assert_eq!(threads::on_card_gone("card-7", None), 0);
    assert_eq!(threads::get("t1").unwrap().state, ThreadState::Awaiting);
}

#[test]
fn an_already_closed_thread_is_not_touched_again() {
    let _g = common::serial();
    fresh();
    awaiting_thread("t1");
    threads::on_card_gone("card-2", None);
    let before = threads::get("t1").unwrap().messages.len();
    assert_eq!(threads::on_card_gone("card-2", None), 0);
    assert_eq!(threads::get("t1").unwrap().messages.len(), before);
}

#[test]
fn closing_a_chat_card_through_the_registry_closes_its_thread() {
    let _g = common::serial();
    fresh();
    awaiting_thread("t1");
    let chat = registry::create_chat_card("t1", "p").unwrap();
    registry::close_card(chat.name).expect("close");
    assert_eq!(
        threads::get("t1").unwrap().state,
        ThreadState::Closed,
        "registry-hooket skal kalde on_card_gone"
    );
}

#[test]
fn batch_close_covers_every_target() {
    let _g = common::serial();
    fresh();
    awaiting_thread("t1");
    let a = registry::create_chat_card("t1", "p").unwrap();
    threads::create_thread("t2", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    let b = registry::create_chat_card("t2", "p").unwrap();
    registry::close_cards(vec![a.name, b.name]).expect("batch close");
    assert_eq!(threads::get("t1").unwrap().state, ThreadState::Closed);
    assert_eq!(threads::get("t2").unwrap().state, ThreadState::Closed);
}
