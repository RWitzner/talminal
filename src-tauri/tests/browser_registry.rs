//! Browser-kort i registryet (plan Task 2). Parallel-robust: asserter kun paa
//! egne kort; aldrig exakte lister eller taeller-absolutter.
use talminal_canvas_lib::registry;

#[test]
fn browser_card_gets_generated_number_and_kind() {
    let info =
        registry::create_browser_card(None, "canvas".into(), "about:blank".into(), String::new())
            .expect("create browser card");
    assert_eq!(info.kind, "browser");
    assert_eq!(info.name, format!("card-{}", info.number));
    assert_eq!(info.opened_by, None);
    assert_eq!(info.url.as_deref(), Some("about:blank"));
    assert!(info.running, "browser-kort foedes alive");
    assert_eq!(info.cwd, "", "browser-kort har ingen cwd");
    registry::close_card(info.name.clone()).expect("close");
}

#[test]
fn pty_paths_reject_browser_cards() {
    let info =
        registry::create_browser_card(None, "canvas".into(), "about:blank".into(), String::new())
            .expect("create");
    let handle = registry::card_handle(&info.name).expect("handle");
    {
        let mut card = handle.lock().expect("lock");
        assert!(
            card.terminal_mut().is_none(),
            "browser har ingen terminal-gren"
        );
        assert_eq!(card.kind_str(), "browser");
    }
    registry::close_card(info.name).expect("close");
}

#[test]
fn expand_close_targets_cascades_owned_browser_cards() {
    // Ejer-terminalen simuleres af et browser-kort-navn — expand er ren
    // navnelogik: opened_by == et input-navn => med i batchen.
    let owner =
        registry::create_browser_card(None, "canvas".into(), "about:blank".into(), String::new())
            .expect("owner-stand-in");
    let owned = registry::create_browser_card(
        Some(owner.name.clone()),
        format!("agent-{}", owner.name),
        "about:blank".into(),
        String::new(),
    )
    .expect("owned");
    let expanded = registry::expand_close_targets(vec![owner.name.clone()]);
    assert!(expanded.contains(&owner.name));
    assert!(
        expanded.contains(&owned.name),
        "kaskaden tager ejede browser-kort med"
    );
    let result = registry::close_cards(expanded).expect("close");
    assert!(result.closed.contains(&owner.name));
    assert!(result.closed.contains(&owned.name));
    assert!(result
        .browser_closed
        .iter()
        .any(|b| b.name == owned.name && b.scope_key == format!("agent-{}", owner.name)));
}

#[test]
fn update_browser_card_tracks_url_title_alive() {
    let info =
        registry::create_browser_card(None, "canvas".into(), "about:blank".into(), String::new())
            .expect("create");
    registry::update_browser_card(
        &info.name,
        Some("https://example.com".into()),
        Some("Example".into()),
        Some(false),
        None,
    )
    .expect("update");
    let listed = registry::list_cards()
        .into_iter()
        .find(|c| c.name == info.name)
        .expect("listed");
    assert_eq!(listed.url.as_deref(), Some("https://example.com"));
    assert_eq!(listed.title.as_deref(), Some("Example"));
    assert!(!listed.running, "alive=false => running=false");
    registry::close_card(info.name).expect("close");
}

#[test]
fn browser_close_target_snapshot_is_read_only() {
    let info = registry::create_browser_card(
        None,
        "canvas".into(),
        "https://example.com".into(),
        String::new(),
    )
    .expect("create");

    let targets =
        registry::browser_close_targets(std::slice::from_ref(&info.name)).expect("snapshot");
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].name, info.name);
    assert_eq!(targets[0].scope_key, "canvas");
    assert!(targets[0].alive);

    let listed = registry::list_cards()
        .into_iter()
        .find(|card| card.name == info.name)
        .expect("preclose snapshot must not detach the card");
    assert!(listed.running, "read-only phase must not mark it dead");

    registry::close_card(info.name).expect("cleanup");
}
