//! Browser-kort webview-host: rene logik-dele (Task 5). Webview-adfaerd er
//! spike-/smoke-daekket (Task 1-verdict + Task 10) — her testes kun URL-
//! politikken og target-diff'en, som er ren funktion uden AppHandle.
//!
//! De faa tests der faktisk roerer det proces-globale REGISTRY tager
//! `common::serial()` — samme defekt som lifecycle.rs' flake 2026-07-26:
//! kortnumre allokeres som laveste ledige, saa navnet `card-N` genopstaar
//! saa snart en samtidig test faar nummer N, og
//! `successful_native_preclose_needs_explicit_registry_phase_to_detach`
//! asserter praecis at navnet er VAEK efter close. Resten af filen er rene
//! funktioner og forbliver parallel.
mod common;

use talminal_canvas_lib::{browser_host, registry};

#[test]
fn url_policy_accepts_http_https_and_blank_only() {
    assert_eq!(
        browser_host::validate_card_url(None).unwrap(),
        "about:blank"
    );
    assert_eq!(
        browser_host::validate_card_url(Some("https://example.com")).unwrap(),
        "https://example.com"
    );
    assert!(browser_host::validate_card_url(Some("file:///c:/x")).is_err());
    assert!(browser_host::validate_card_url(Some("javascript:alert(1)")).is_err());
    assert!(browser_host::validate_card_url(Some("not a url")).is_err());
}

#[test]
fn new_target_diff_identifies_created_page() {
    let before = vec![("T1".to_string(), "about:blank".to_string())];
    let after = vec![
        ("T1".to_string(), "about:blank".to_string()),
        ("T2".to_string(), "https://example.com/".to_string()),
    ];
    assert_eq!(
        browser_host::new_target_id(&before, &after).as_deref(),
        Some("T2")
    );
    assert_eq!(browser_host::new_target_id(&after, &after), None);
}

// Final-review Finding 2: an empty before-snapshot must be a HARD error, never
// a diff basis. With before=[], new_target_id would return after's FIRST target
// (the keeper's about:blank) as "the new card" — mis-tagging the card with the
// keeper's target and letting the poller /json/close the page the agent drives.
#[test]
fn empty_before_snapshot_is_rejected() {
    // Empty before ⇒ Err (the dangerous case create_card_webview must refuse).
    assert!(browser_host::before_snapshot_ok(&[]).is_err());

    // Demonstrate WHY: against an empty before, the diff would hand back the
    // keeper's id — exactly what the guard prevents upstream.
    let after = vec![
        ("KEEPER".to_string(), "about:blank".to_string()),
        ("CARD".to_string(), "https://example.com/".to_string()),
    ];
    assert_eq!(
        browser_host::new_target_id(&[], &after).as_deref(),
        Some("KEEPER")
    );

    // A non-empty before (keeper listed) ⇒ Ok, and the diff finds the real card.
    let before = vec![("KEEPER".to_string(), "about:blank".to_string())];
    assert!(browser_host::before_snapshot_ok(&before).is_ok());
    assert_eq!(
        browser_host::new_target_id(&before, &after).as_deref(),
        Some("CARD")
    );
}

// Owner decision 2026-07-20: scope death requires 2 CONSECUTIVE poll
// failures (amends Task 5's single-failure death path — one slow /json
// read must not mass-kill a healthy scope).

#[test]
fn scope_failure_debounce_requires_two_consecutive_failures() {
    use std::collections::HashMap;
    let mut failures: HashMap<String, u32> = HashMap::new();

    // First failure: cards stay alive.
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    ));
    // Second CONSECUTIVE failure: today's death path fires.
    assert!(browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    ));
}

#[test]
fn scope_failure_debounce_resets_streak_on_success() {
    use std::collections::HashMap;
    let mut failures: HashMap<String, u32> = HashMap::new();

    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    )); // failure
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        false,
        true
    )); // success resets
        // A fresh failure after a success is only the first of a new streak.
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    ));
}

#[test]
fn scope_failure_debounce_tracks_scopes_independently() {
    use std::collections::HashMap;
    let mut failures: HashMap<String, u32> = HashMap::new();

    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    )); // a: streak 1
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-b",
        true,
        true
    )); // b: streak 1
    assert!(browser_host::track_scope_failure(
        &mut failures,
        "agent-b",
        true,
        true
    )); // b: streak 2, dead
        // agent-a's streak must be untouched by agent-b's calls: a success now
        // clears it with no death, proving it was still at 1 (not already dead).
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        false,
        true
    ));
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    )); // new streak: 1st again
}

// P1 lazy-keeper regression (GPT Del B review find, 2026-07-21): a scope is
// registered in SCOPES at worker-spawn WITHOUT a keeper (lazy window), so its
// CDP endpoint is EXPECTEDLY down. The poller's Err arm must not accumulate a
// failure streak during that window — otherwise the FIRST card opened after
// keeper-up inherits streak >= 2 and dies on a SINGLE slow /json read,
// de-facto bypassing the owner-locked 2-consecutive-failure debounce.

#[test]
fn keeperless_scope_accumulates_no_failure_streak() {
    use std::collections::HashMap;
    let mut failures: HashMap<String, u32> = HashMap::new();

    // Entire lazy window: CDP down every 2s tick, no keeper registered.
    // Never dead, and the streak stays cleared (not merely frozen).
    for _ in 0..50 {
        assert!(!browser_host::track_scope_failure(
            &mut failures,
            "agent-a",
            true,
            false
        ));
    }
    assert!(!failures.contains_key("agent-a"));
}

#[test]
fn fresh_card_after_keeper_up_survives_single_json_failure() {
    use std::collections::HashMap;
    let mut failures: HashMap<String, u32> = HashMap::new();

    // Lazy window: many Err ticks while the scope has no keeper.
    for _ in 0..10 {
        browser_host::track_scope_failure(&mut failures, "agent-a", true, false);
    }
    // Keeper comes up and the first card opens; the fresh browser is loading
    // its first page and ONE /json read is slow. Streak must be 1 — the card
    // survives (the 2-failure debounce genuinely applies to the first card).
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    ));
    // The owner-locked debounce itself is unchanged: a SECOND consecutive
    // failure with the keeper present still fires the death path.
    assert!(browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    ));
}

#[test]
fn keeper_loss_mid_streak_clears_rather_than_freezes() {
    use std::collections::HashMap;
    let mut failures: HashMap<String, u32> = HashMap::new();

    // Streak 1 with keeper present, then the keeper id is cleared (heal's
    // teardown window — teardown itself dead-marks the cards, so the poller
    // must not carry the old streak into the NEXT keeper era).
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    ));
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        false
    ));
    assert!(!failures.contains_key("agent-a"));
    // Fresh keeper era: the first failure is streak 1 again, not death.
    assert!(!browser_host::track_scope_failure(
        &mut failures,
        "agent-a",
        true,
        true
    ));
}

#[test]
fn native_close_failure_keeps_browser_card_live_and_retryable() {
    let _serial = common::serial();
    let info = registry::create_browser_card(
        None,
        "canvas".into(),
        "https://example.com".into(),
        String::new(),
    )
    .expect("create");

    let outcome =
        browser_host::preclose_browser_cards_with(std::slice::from_ref(&info.name), |_target| {
            Err("synthetic native close failure".to_string())
        })
        .expect("preclose result");

    assert!(outcome.closed.is_empty());
    assert_eq!(outcome.errors.len(), 1);
    assert!(outcome.errors[0]
        .message
        .contains("synthetic native close failure"));
    let listed = registry::list_cards()
        .into_iter()
        .find(|card| card.name == info.name)
        .expect("native failure must preserve the registry entry");
    assert!(
        listed.running,
        "failed native close must leave the card live for a retry"
    );

    registry::close_card(info.name).expect("cleanup");
}

#[test]
fn successful_native_preclose_needs_explicit_registry_phase_to_detach() {
    let _serial = common::serial();
    let info = registry::create_browser_card(
        None,
        "canvas".into(),
        "https://example.com".into(),
        String::new(),
    )
    .expect("create");

    let outcome =
        browser_host::preclose_browser_cards_with(std::slice::from_ref(&info.name), |_target| {
            Ok(())
        })
        .expect("preclose");
    assert_eq!(outcome.closed, vec![info.name.clone()]);
    assert!(outcome.errors.is_empty());

    let listed = registry::list_cards()
        .into_iter()
        .find(|card| card.name == info.name)
        .expect("phase 1 must keep a retryable registry entry");
    assert!(
        !listed.running,
        "accepted native close is mirrored as a dead card before detach"
    );

    let result = registry::close_cards(vec![info.name.clone()]).expect("registry phase");
    assert_eq!(result.closed, vec![info.name.clone()]);
    assert!(
        registry::card_handle(&info.name).is_err(),
        "phase 2 is the only operation that detaches the card"
    );
}

#[test]
fn partial_native_batch_failure_detaches_nothing() {
    let _serial = common::serial();
    let first = registry::create_browser_card(
        None,
        "canvas".into(),
        "https://example.com/first".into(),
        String::new(),
    )
    .expect("first");
    let second = registry::create_browser_card(
        None,
        "canvas".into(),
        "https://example.com/second".into(),
        String::new(),
    )
    .expect("second");
    let names = vec![first.name.clone(), second.name.clone()];

    let outcome = browser_host::preclose_browser_cards_with(&names, |target| {
        if target.name == second.name {
            Err("second webview refused close".to_string())
        } else {
            Ok(())
        }
    })
    .expect("preclose");

    assert_eq!(outcome.closed, vec![first.name.clone()]);
    assert_eq!(outcome.errors.len(), 1);
    let cards = registry::list_cards();
    let first_after = cards
        .iter()
        .find(|card| card.name == first.name)
        .expect("successfully preclosed entry stays addressable");
    let second_after = cards
        .iter()
        .find(|card| card.name == second.name)
        .expect("failed entry stays addressable");
    assert!(!first_after.running, "successful half is marked dead");
    assert!(second_after.running, "failed half remains live for retry");

    registry::close_cards(names).expect("cleanup");
}

#[test]
fn split_close_batch_keeps_terminals_when_browser_preclose_fails() {
    // Kaskade-regressionen: en wedged browser-webview maa ikke holde sin
    // ejer-terminal aaben. Preclose-fejlede navne pilles ud af registry-
    // batchen; resten lukkes faerdigt.
    let names = vec![
        "agent-3".to_string(),
        "card-7".to_string(),
        "card-9".to_string(),
    ];
    let errors = vec![registry::CloseCardsError {
        name: "card-7".to_string(),
        message: "native webview close failed: wedged".to_string(),
    }];
    let (to_close, skipped) = browser_host::split_close_batch(names, &errors);
    assert_eq!(to_close, vec!["agent-3".to_string(), "card-9".to_string()]);
    assert_eq!(skipped, vec!["card-7".to_string()]);

    // Ingen fejl => hele batchen gaar videre uroert.
    let (all, none) = browser_host::split_close_batch(vec!["agent-3".to_string()], &[]);
    assert_eq!(all, vec!["agent-3".to_string()]);
    assert!(none.is_empty());
}

// ---------------------------------------------------------------------------
// Keeper-haerdning (2026-07-20 dogfood-bug: playwright navigerede keeperen,
// polleren draebte den, scopet doede permanent, og et ghost-dead-card blev
// staaende i frontenden). Adversarielt reviewet design: keeper-immunitet er
// TARGET-ID-baseret, scope self-heal i fast-path, cleanup emitter altid.
// ---------------------------------------------------------------------------

#[test]
fn identify_keeper_prefers_sentinel_then_requires_single_page() {
    // Sentinel-match vinder uanset raekkefoelge og antal sider.
    let sentinel_pages = vec![
        ("CARD".to_string(), "https://example.com/".to_string()),
        (
            "KEEPER".to_string(),
            "about:blank#keeper-agent-card-1".to_string(),
        ),
    ];
    assert_eq!(
        browser_host::identify_keeper(&sentinel_pages, "agent-card-1").unwrap(),
        "KEEPER"
    );

    // Fallback (fragment strippet af /json): praecis én side er entydig.
    let single = vec![("ONLY".to_string(), "about:blank".to_string())];
    assert_eq!(
        browser_host::identify_keeper(&single, "agent-card-1").unwrap(),
        "ONLY"
    );

    // Flere sider uden sentinel-match er en HARD fejl — aldrig "gaet den
    // foerste" (samme fejlklasse som final-review Finding 2's mis-tagging).
    let ambiguous = vec![
        ("A".to_string(), "about:blank".to_string()),
        ("B".to_string(), "about:blank".to_string()),
    ];
    assert!(browser_host::identify_keeper(&ambiguous, "agent-card-1").is_err());
    assert!(browser_host::identify_keeper(&[], "agent-card-1").is_err());

    // Spoof-vaern (review F3): sentinel-markoeren taeller kun paa
    // about:blank-URLer — en http-side med `#keeper-...` i fragmentet maa
    // ikke kunne tilrane sig keeper-identiteten (og dermed drabs-immunitet).
    let spoof = vec![
        (
            "EVIL".to_string(),
            "https://x.example/#keeper-agent-card-1".to_string(),
        ),
        (
            "KEEPER".to_string(),
            "about:blank#keeper-agent-card-1".to_string(),
        ),
    ];
    assert_eq!(
        browser_host::identify_keeper(&spoof, "agent-card-1").unwrap(),
        "KEEPER"
    );
    let spoof_only = vec![
        (
            "EVIL".to_string(),
            "https://x.example/#keeper-agent-card-1".to_string(),
        ),
        ("OTHER".to_string(), "about:blank".to_string()),
    ];
    assert!(
        browser_host::identify_keeper(&spoof_only, "agent-card-1").is_err(),
        "spoof + blank uden sentinel = ambiguity, aldrig spoof-match"
    );
}

#[test]
fn reconcile_without_keeper_tid_documents_residual_kill_risk() {
    // Review F6 (dokumenteret rest-risiko): er keeper-tid'et IKKE
    // registreret (fx create_keeper_webview fejlede efter add_child, foer
    // fast-path-reparationen i ensure_scope_ready naar at koere), er en
    // KAPRET keeper (non-blank URL) uadskillelig fra et rogue-target og
    // lukkes efter 2 sightings — som foer fixet. Vaernet er reparationen i
    // ensure_scope_ready (samme CREATE_LOCK), ikke denne funktion. Testen
    // fryser rest-adfaerden, saa en aendring her er et bevidst valg.
    use std::collections::{HashMap, HashSet};
    let cards: Vec<(String, String, bool)> = Vec::new();
    let hijacked = vec![("KEEPER".to_string(), "https://github.com/".to_string())];
    let mut pending = HashMap::new();
    let mut seen = HashSet::new();

    let first = browser_host::reconcile_actions(&cards, &hijacked, None, &mut pending, &mut seen);
    assert!(first.close_targets.is_empty(), "foerste sighting: naade");
    assert!(!first.renavigate_keeper, "uden tid kendes keeperen ikke");

    let second = browser_host::reconcile_actions(&cards, &hijacked, None, &mut pending, &mut seen);
    assert_eq!(second.close_targets, vec!["KEEPER".to_string()]);
}

#[test]
fn reconcile_never_closes_keeper_and_requests_renavigation_when_hijacked() {
    use std::collections::{HashMap, HashSet};
    let cards: Vec<(String, String, bool)> = Vec::new();
    let hijacked = vec![("KEEPER".to_string(), "https://github.com/".to_string())];
    let mut pending = HashMap::new();
    let mut seen = HashSet::new();

    // Selv efter mange sightings er keeper-target'et immunt mod /json/close;
    // en kapret (non-blank) keeper renavigeres i stedet for at draebes.
    for sighting in 0..10 {
        let actions = browser_host::reconcile_actions(
            &cards,
            &hijacked,
            Some("KEEPER"),
            &mut pending,
            &mut seen,
        );
        assert!(
            actions.close_targets.is_empty(),
            "sighting {sighting}: keeper maa aldrig lukkes"
        );
        assert!(
            actions.renavigate_keeper,
            "sighting {sighting}: kapret keeper skal renavigeres"
        );
    }

    // En keeper paa about:blank (fragment strippet eller gendannet) roeres ikke.
    let blank = vec![("KEEPER".to_string(), "about:blank".to_string())];
    let actions =
        browser_host::reconcile_actions(&cards, &blank, Some("KEEPER"), &mut pending, &mut seen);
    assert!(!actions.renavigate_keeper);
    assert!(actions.close_targets.is_empty());
}

#[test]
fn reconcile_keeps_blank_exemption_and_two_sighting_grace_for_unknowns() {
    use std::collections::{HashMap, HashSet};
    let cards = vec![("card-1".to_string(), "T-CARD".to_string(), true)];
    // Keeperens eget target SKAL vaere i samplet: siden M2 (2026-07-29)
    // afviser browser_host::trusted_sample et keeper-loest svar FOER
    // reconcile_actions kaldes, saa et sample uden T-KEEPER ville maale et
    // scenarie produktionen ikke kan komme i.
    let pages = vec![
        (
            "T-KEEPER".to_string(),
            "about:blank#keeper-agent-a".to_string(),
        ),
        ("T-CARD".to_string(), "https://example.com/".to_string()),
        ("T-BLANK".to_string(), "about:blank".to_string()),
        ("T-ROGUE".to_string(), "https://rogue.example/".to_string()),
    ];
    assert!(
        browser_host::trusted_sample(Ok(pages.clone()), Some("T-KEEPER")).is_some(),
        "praemissen skal vaere naaebar for produktionen"
    );
    let mut pending = HashMap::new();
    let mut seen = HashSet::new();

    let first =
        browser_host::reconcile_actions(&cards, &pages, Some("T-KEEPER"), &mut pending, &mut seen);
    assert!(
        first.close_targets.is_empty(),
        "foerste sighting: naadesvindue"
    );
    assert!(first.dead_names.is_empty(), "kortets target lever");

    let second =
        browser_host::reconcile_actions(&cards, &pages, Some("T-KEEPER"), &mut pending, &mut seen);
    assert_eq!(
        second.close_targets,
        vec!["T-ROGUE".to_string()],
        "anden sighting lukker rogue-tabben — aldrig den blanke"
    );
}

#[test]
fn reconcile_marks_disappeared_targets_dead_but_spares_mid_creation_cards() {
    use std::collections::{HashMap, HashSet};
    let cards = vec![
        ("card-1".to_string(), "GONE".to_string(), true),
        // Mid-creation: registry-kortet findes, men target er endnu ikke sat.
        ("card-2".to_string(), String::new(), true),
        // Allerede doedt kort maa ikke re-rapporteres.
        ("card-3".to_string(), "DEAD-ALREADY".to_string(), false),
    ];
    let pages = vec![("KEEPER".to_string(), "about:blank".to_string())];
    let mut pending = HashMap::new();
    let mut seen = HashSet::new();
    let actions =
        browser_host::reconcile_actions(&cards, &pages, Some("KEEPER"), &mut pending, &mut seen);
    assert_eq!(actions.dead_names, vec!["card-1".to_string()]);
}

#[test]
fn scope_death_grants_bounded_grace_to_cards_without_target() {
    use std::collections::HashMap;
    let cards = vec![
        ("card-tid".to_string(), "T1".to_string(), true),
        ("card-fresh".to_string(), String::new(), true),
    ];
    let mut grace = HashMap::new();

    // Tick 1-4: kort MED target doer straks; mid-creation-kortet skaanes
    // (naadesvinduet er stoerre end worst-case-oprettelsen ~7 s).
    for tick in 1..=4 {
        let dead = browser_host::scope_dead_names(&cards, &mut grace, 4);
        assert!(dead.contains(&"card-tid".to_string()), "tick {tick}");
        assert!(
            !dead.contains(&"card-fresh".to_string()),
            "tick {tick}: naadesvindue for tomt target"
        );
    }
    // Tick 5: naade opbrugt — et evigt target-loest kort maa ikke laekke.
    let dead = browser_host::scope_dead_names(&cards, &mut grace, 4);
    assert!(dead.contains(&"card-fresh".to_string()));

    // Allerede doede kort rapporteres aldrig.
    let dead_cards = vec![("card-x".to_string(), String::new(), false)];
    assert!(browser_host::scope_dead_names(&dead_cards, &mut HashMap::new(), 4).is_empty());
}

#[test]
fn fastpath_heal_orders_teardown_wait_recreate_and_skips_when_healthy() {
    use std::cell::RefCell;
    use std::time::Duration;
    let order: RefCell<Vec<&'static str>> = RefCell::new(Vec::new());

    // Sund probe ⇒ genbrug; ingen af heal-stadierne roeres.
    let healed = browser_host::fastpath_heal_with(
        || true,
        3,
        Duration::ZERO,
        || {
            order.borrow_mut().push("teardown");
            Ok(())
        },
        || {
            order.borrow_mut().push("wait");
            true
        },
        || {
            order.borrow_mut().push("recreate");
            Ok(())
        },
    )
    .expect("healthy");
    assert!(!healed);
    assert!(order.borrow().is_empty());

    // Doed probe ⇒ teardown → wait → recreate, i praecis den raekkefoelge.
    let healed = browser_host::fastpath_heal_with(
        || false,
        3,
        Duration::ZERO,
        || {
            order.borrow_mut().push("teardown");
            Ok(())
        },
        || {
            order.borrow_mut().push("wait");
            true
        },
        || {
            order.borrow_mut().push("recreate");
            Ok(())
        },
    )
    .expect("healed");
    assert!(healed);
    assert_eq!(*order.borrow(), vec!["teardown", "wait", "recreate"]);
}

#[test]
fn fastpath_heal_failure_paths_are_hard_errors() {
    use std::time::Duration;

    // Zombie-webview/port frigives aldrig ⇒ hard fejl m. runbook-anvisning.
    let err = browser_host::fastpath_heal_with(
        || false,
        1,
        Duration::ZERO,
        || Ok(()),
        || false,
        || Ok(()),
    )
    .unwrap_err();
    assert!(err.contains("respawn"), "runbook-anvisning mangler: {err}");

    // Recreate-fejl propagerer (aldrig stille degradering).
    let err = browser_host::fastpath_heal_with(
        || false,
        1,
        Duration::ZERO,
        || Ok(()),
        || true,
        || Err("keeper add_child failed".to_string()),
    )
    .unwrap_err();
    assert!(err.contains("keeper add_child failed"));
}

#[test]
fn failed_open_cleanup_detaches_before_emitting_dead() {
    // Ghost-kort-regressionen: registry-fjernelsen SKAL ske FOER dead-emitten,
    // saa frontendens event-udloeste refresh aldrig kan naa at gense det
    // halvfaerdige kort. Sømmen testes med injiceret close — navnebaserede
    // registry-asserts er racy under parallel test (kort-nummer-genbruget kan
    // genudlevere navnet til en anden test mellem close og assert).
    use std::cell::RefCell;
    let order: RefCell<Vec<&'static str>> = RefCell::new(Vec::new());

    browser_host::cleanup_failed_open_with(
        || {
            order.borrow_mut().push("close");
            Ok(())
        },
        |names| {
            order.borrow_mut().push("emit");
            assert_eq!(names, vec!["card-x".to_string()]);
        },
        "card-x",
    );
    assert_eq!(*order.borrow(), vec!["close", "emit"]);

    // Emitten fyrer OGSAA naar close fejler (kortet kan allerede vaere vaek):
    // et event for meget er en no-op-refresh, et for lidt er et ghost-kort.
    order.borrow_mut().clear();
    browser_host::cleanup_failed_open_with(
        || {
            order.borrow_mut().push("close-err");
            Err("no such card".to_string())
        },
        |_names| order.borrow_mut().push("emit"),
        "card-x",
    );
    assert_eq!(*order.borrow(), vec!["close-err", "emit"]);
}
