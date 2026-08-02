use std::net::TcpListener;

use talminal_canvas_lib::{browser, browser_host};

#[test]
fn ensure_scope_is_idempotent_and_ports_differ_across_scopes() {
    let a1 = browser::ensure_scope(Some("card-901")).expect("a1");
    let a2 = browser::ensure_scope(Some("card-901")).expect("a2");
    assert_eq!(
        a1.port, a2.port,
        "samme scope => samme port (endpoint-uforanderlighed)"
    );
    assert_eq!(a1.key, "agent-card-901");
    let b = browser::ensure_scope(Some("card-902")).expect("b");
    assert_ne!(a1.port, b.port, "scopes deler aldrig port");
    let canvas = browser::ensure_scope(None).expect("canvas");
    assert_eq!(canvas.key, "canvas");
    browser::remove_scope(&a1.key);
    browser::remove_scope(&b.key);
    // canvas-scopet ryddes ikke (delt) — remove er kun for agent-scopes i test-hygiejne.
}

#[test]
fn profile_dir_is_launch_and_scope_scoped() {
    let s = browser::ensure_scope(Some("card-903")).expect("scope");
    let path = s.profile_dir.display().to_string();
    assert!(path.contains(browser::launch_id()), "run-scopet: {path}");
    assert!(path.ends_with("agent-card-903"), "scope-mappen: {path}");
    browser::remove_scope(&s.key);
}

#[test]
fn browser_args_repeat_wry_defaults_and_add_ours() {
    let args = browser::additional_browser_args(9345);
    assert!(args.contains("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"));
    assert!(args.contains("--remote-debugging-port=9345"));
    assert!(args.contains("--autoplay-policy=user-gesture-required"));
}

#[test]
fn visibility_composition_matches_spec_8a() {
    use talminal_canvas_lib::browser::webview_should_show as show;
    assert!(show(false, None, "card-1", true));
    assert!(!show(true, None, "card-1", true), "occlusion skjuler alt");
    assert!(
        show(false, Some("card-1"), "card-1", true),
        "fuldskaerms-kortet vises"
    );
    assert!(
        !show(false, Some("card-1"), "card-2", true),
        "andre skjules under fuldskaerm"
    );
    assert!(
        !show(true, Some("card-1"), "card-1", true),
        "modaler vinder over fuldskaerm"
    );
}

// Keeper-haerdning (2026-07-20): doede korts zombie-webviews maa aldrig
// daekke dead-chromen ("Browserprocessen er doed — luk kortet") i DOM'en.
#[test]
fn visibility_composition_hides_dead_cards() {
    use talminal_canvas_lib::browser::webview_should_show as show;
    assert!(
        !show(false, None, "card-1", false),
        "doedt kort vises aldrig"
    );
    assert!(
        !show(false, Some("card-1"), "card-1", false),
        "doed vinder over fuldskaerm"
    );
}

#[test]
fn keeper_identity_is_stored_on_the_scope() {
    let s = browser::ensure_scope(Some("card-904")).expect("scope");
    assert!(
        s.keeper_target_id.is_none(),
        "nyt scope: intet keeper-target endnu"
    );
    browser::set_keeper_target(&s.key, Some("KEEPER-TID".to_string()));
    let scopes = browser::all_scopes();
    let found = scopes
        .iter()
        .find(|x| x.key == s.key)
        .expect("scope listet");
    assert_eq!(found.keeper_target_id.as_deref(), Some("KEEPER-TID"));
    browser::remove_scope(&s.key);
}

#[test]
fn keeper_sentinel_is_blank_with_scope_fragment() {
    // Sentinel-URL'en identificerer keeperen strukturelt i /json (og bevarer
    // about:blank-praefikset, saa pollerens blank-fritagelse ogsaa daekker den).
    let sentinel = browser::keeper_sentinel("agent-card-1");
    assert_eq!(sentinel, "about:blank#keeper-agent-card-1");
}

#[test]
fn lazy_keeper_refuses_a_stolen_cdp_port_without_reassigning_endpoint() {
    let thief = TcpListener::bind(("127.0.0.1", 0)).expect("bind synthetic port thief");
    let port = thief.local_addr().expect("thief address").port();

    let error = browser_host::reserve_exact_cdp_port(port)
        .expect_err("an occupied lazy endpoint must fail closed");
    assert!(error.contains(&port.to_string()));
    assert!(error.contains("opret det igen"));

    drop(thief);
    let guard = browser_host::reserve_exact_cdp_port(port)
        .expect("the exact same endpoint becomes usable after release");
    assert_eq!(guard.local_addr().expect("guard address").port(), port);
}
