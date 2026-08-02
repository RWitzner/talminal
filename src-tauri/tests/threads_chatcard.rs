mod common;

use talminal_canvas_lib::registry;

#[test]
fn a_chat_card_reports_kind_chat_and_carries_thread_and_purpose() {
    let _g = common::serial();
    let info = registry::create_chat_card("t7", "spar om submit").unwrap();
    assert_eq!(info.kind, "chat");
    assert_eq!(info.thread_id.as_deref(), Some("t7"));
    assert_eq!(info.purpose.as_deref(), Some("spar om submit"));
    assert_eq!(info.profile, "", "et chat-kort har ingen agent-profil");
    assert_eq!(info.cwd, "", "og ingen arbejdsmappe");
    assert!(info.running, "kortet lever saa laenge traaden findes");
    let listed = registry::list_cards()
        .into_iter()
        .find(|c| c.name == info.name)
        .expect("chat-kortet er i listen");
    assert_eq!(listed.kind, "chat");
    registry::close_card(info.name).ok();
}

#[test]
fn a_terminal_card_still_reports_none_for_the_new_fields() {
    let _g = common::serial();
    let info = registry::create_chat_card("t7b", "p").unwrap();
    let others: Vec<_> = registry::list_cards()
        .into_iter()
        .filter(|c| c.kind == "terminal")
        .collect();
    for card in others {
        assert!(card.thread_id.is_none(), "terminal-kort baerer ingen traad");
        assert!(card.purpose.is_none());
    }
    registry::close_card(info.name).ok();
}

#[test]
fn a_chat_card_has_no_terminal_and_no_browser_runtime() {
    let _g = common::serial();
    let info = registry::create_chat_card("t8", "p").unwrap();
    let handle = registry::card_handle(&info.name).unwrap();
    {
        let mut guard = handle.lock().unwrap();
        assert!(guard.terminal().is_none(), "chat-kort har ingen PTY");
        assert!(guard.browser_mut().is_none(), "chat-kort har intet webview");
        assert_eq!(guard.kind_str(), "chat");
        assert_eq!(guard.name(), info.name);
    }
    registry::close_card(info.name).ok();
}

#[test]
fn chat_thread_id_is_only_set_for_chat_cards() {
    let _g = common::serial();
    let info = registry::create_chat_card("t9", "p").unwrap();
    assert_eq!(registry::chat_thread_id(&info.name).as_deref(), Some("t9"));
    assert!(registry::chat_thread_id("card-does-not-exist").is_none());
    registry::close_card(info.name).ok();
}

#[test]
fn submit_prompt_to_a_chat_card_names_the_actual_kind() {
    let _g = common::serial();
    let info = registry::create_chat_card("t10", "p").unwrap();
    let err =
        talminal_canvas_lib::submit::submit_prompt(info.name.clone(), "x".into()).unwrap_err();
    assert!(
        err.contains("chat"),
        "fejlen skal navngive korttypen, ikke sige 'browser': {err}"
    );
    registry::close_card(info.name).ok();
}
