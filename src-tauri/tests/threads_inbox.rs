mod common;

use talminal_canvas_lib::threads::policy::AcceptsFromView;
use talminal_canvas_lib::threads::{self, FromKind, Intent, PostRequest};

fn seeded(count: usize) {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    for index in 0..count {
        threads::post(PostRequest {
            thread: "t1".into(),
            from_card: "card-1".into(),
            from_kind: FromKind::Agent,
            intent: Intent::Sparring,
            text: format!("besked {index}"),
        })
        .unwrap();
    }
}

#[test]
fn the_same_batch_is_redelivered_until_acked() {
    let _g = common::serial();
    seeded(2);
    let first = threads::inbox_take("t1", "card-2", None).unwrap();
    let second = threads::inbox_take("t1", "card-2", None).unwrap();
    assert_eq!(first.messages.len(), 2);
    assert_eq!(
        first
            .messages
            .iter()
            .map(|message| message.seq)
            .collect::<Vec<_>>(),
        second
            .messages
            .iter()
            .map(|message| message.seq)
            .collect::<Vec<_>>()
    );
    let third = threads::inbox_take("t1", "card-2", Some(first.batch_id)).unwrap();
    assert!(third.messages.is_empty());
}

#[test]
fn a_batch_caps_at_five_and_reports_has_more() {
    let _g = common::serial();
    seeded(7);
    let first = threads::inbox_take("t1", "card-2", None).unwrap();
    assert_eq!(first.messages.len(), threads::INBOX_BATCH_MAX);
    assert!(first.has_more);
    assert_eq!(first.batch_id, 5);

    let second = threads::inbox_take("t1", "card-2", Some(first.batch_id)).unwrap();
    assert_eq!(second.messages.len(), 2);
    assert!(!second.has_more);
    assert_eq!(second.messages[0].seq, 6);
}

#[test]
fn the_cursor_is_per_recipient() {
    let _g = common::serial();
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: threads::OWNER.into(),
        from_kind: FromKind::Human,
        intent: Intent::Sparring,
        text: "til begge".into(),
    })
    .unwrap();
    let first = threads::inbox_take("t1", "card-1", None).unwrap();
    threads::inbox_take("t1", "card-1", Some(first.batch_id)).unwrap();
    let second = threads::inbox_take("t1", "card-2", None).unwrap();
    assert_eq!(second.messages.len(), 1);
}

#[test]
fn a_non_member_cannot_read_the_inbox() {
    let _g = common::serial();
    seeded(1);
    let err = threads::inbox_take("t1", "card-9", None).unwrap_err();
    assert!(err.contains("not a member"), "{err}");
}

#[test]
fn an_unknown_thread_is_an_error_not_a_panic() {
    let _g = common::serial();
    seeded(1);
    assert!(threads::inbox_take("nope", "card-2", None).is_err());
}

#[test]
fn an_empty_inbox_yields_batch_id_zero_and_no_messages() {
    let _g = common::serial();
    seeded(0);
    let batch = threads::inbox_take("t1", "card-2", None).unwrap();
    assert!(batch.messages.is_empty());
    assert!(!batch.has_more);
    assert_eq!(batch.batch_id, 0);
}
