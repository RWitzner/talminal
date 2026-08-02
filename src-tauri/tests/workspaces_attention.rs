mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};
use talminal_canvas_lib::registry;
use talminal_canvas_lib::workspaces::{self, attention::CardAttention};

/// `sync_initial_visibility` er kun korrekt fordi `set_attention_visibility`
/// stempler kanten FOER den itererer registryet: en maskine der installeres et
/// oejeblik for sent til iterationen, skal kunne laese kanten selv.
///
/// Testen holder ét korts laas, saa iterationen er fanget, og kraever at
/// stemplet allerede er sat imens.
#[test]
fn kanten_stemples_foer_registryet_itereres() {
    let _g = common::serial();
    assert_eq!(
        workspaces::attention_visibility_now(),
        None,
        "ingen kant er faldet endnu i denne testbinary"
    );

    let cwd = tempfile::tempdir().unwrap();
    let kort = registry::create_card(cwd.path().display().to_string(), "claude".to_string(), None)
        .expect("create_card");
    let handle = registry::card_handle(&kort.name).expect("card_handle");
    {
        let mut guard = handle.lock().unwrap_or_else(|p| p.into_inner());
        guard.terminal_mut().expect("terminal-kort").attention =
            Some(Arc::new(CardAttention::new(&[])));
    }

    // Staar for `spawn_into`, der holder kortets laas hen over PTY-spawnet.
    let spawn_laas = handle.lock().unwrap_or_else(|p| p.into_inner());
    let kant = std::thread::spawn(|| workspaces::set_attention_visibility(true));

    let deadline = Instant::now() + Duration::from_secs(5);
    while workspaces::attention_visibility_now().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        workspaces::attention_visibility_now(),
        Some(true),
        "kanten skal vaere stemplet FOER iterationen — ellers kan et kort der \
         installeres et oejeblik senere ikke laese den"
    );
    assert!(
        !kant.is_finished(),
        "iterationen skulle stadig vaere fanget paa kortets laas — beviset for \
         at stemplet kom foerst"
    );

    drop(spawn_laas);
    kant.join().unwrap();
    assert_eq!(workspaces::attention_visibility_now(), Some(true));

    workspaces::set_attention_visibility(false);
    assert_eq!(workspaces::attention_visibility_now(), Some(false));

    let _ = registry::close_cards(vec![kort.name]);
}
