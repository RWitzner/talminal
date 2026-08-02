// Task 14 — keyring-secrets mod den AEGTE Windows Credential Manager
// (service "Talminal", keyring 3 m. feature "windows-native" — ingen mock).
//
// Secrets-testcases (planens Step 1):
//   (1) store→load roundtrip (inkl. overskrivning + non-ASCII-vaerdi)
//   (2) load af ukendt noegle → Ok(None)
//   (3) delete→load → Ok(None) (og delete er idempotent)
//
// Settings-testcases (Task 14's workspace.json-del) er FLYTTET til
// tests/workspace.rs under B-light T4 (global settings.json +
// WorkspaceResponse-DTO) — se settings_bor_globalt_ikke_i_workspace_json m.fl.
//
// VIGTIGT — secrets-testene skriver i brugerens rigtige Credential Manager:
//   - ALDRIG produktionsnavnene "stt_api_key"/"router_api_key"; alle navne er
//     test-specifikke og unikke pr. koersel (pid + nanos + taeller).
//   - Hver test rydder op efter sig selv via en Drop-guard, som ogsaa koerer
//     ved assert-panik (teardown-kravet).

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use talminal_canvas_lib::secrets::{delete_secret, load_secret, store_secret};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Unikt test-noeglenavn: kolliderer hverken med produktionsnoegler, andre
/// tests i samme koersel eller parallelle cargo-koersler (begge feature-states).
fn test_key(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .subsec_nanos();
    format!(
        "talminal-test-{tag}-{}-{}-{nanos}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Teardown-guard: sletter test-noeglen ved scope-exit — ogsaa naar testen
/// panikker undervejs. Fejl ignoreres (noeglen kan allerede vaere slettet).
struct KeyGuard(String);

impl Drop for KeyGuard {
    fn drop(&mut self) {
        let _ = delete_secret(self.0.clone());
    }
}

#[test]
fn store_then_load_roundtrips() {
    let key = test_key("roundtrip");
    let _guard = KeyGuard(key.clone());

    store_secret(key.clone(), "sk-test-vaerdi-æøå-1".to_string()).expect("store_secret");
    assert_eq!(
        load_secret(key.clone()).expect("load_secret"),
        Some("sk-test-vaerdi-æøå-1".to_string()),
        "roundtrip skal give praecis den gemte vaerdi"
    );

    // Overskrivning: nyeste vaerdi vinder (Settings-panelets erstat-flow).
    store_secret(key.clone(), "sk-test-vaerdi-2".to_string()).expect("re-store");
    assert_eq!(
        load_secret(key).expect("load efter overskrivning"),
        Some("sk-test-vaerdi-2".to_string())
    );
}

#[test]
fn load_of_unknown_key_is_ok_none() {
    // Aldrig gemt => Ok(None) — IKKE en fejl (panelets "ikke sat"-tilstand).
    let key = test_key("unknown");
    assert_eq!(
        load_secret(key).expect("load_secret af ukendt noegle"),
        None
    );
}

#[test]
fn delete_then_load_is_ok_none() {
    let key = test_key("delete");
    let _guard = KeyGuard(key.clone());

    store_secret(key.clone(), "slettes-straks".to_string()).expect("store_secret");
    delete_secret(key.clone()).expect("delete_secret");
    assert_eq!(
        load_secret(key.clone()).expect("load efter delete"),
        None,
        "slettet noegle skal loade som Ok(None)"
    );

    // Idempotens: delete af allerede-slettet noegle er Ok(()) — panelets
    // ryd-knap maa ikke fejle ved dobbeltklik.
    delete_secret(key).expect("delete af allerede-slettet noegle");
}

#[test]
fn migration_copies_verifies_then_deletes() {
    let old_a = test_key("mig-old-a");
    let old_b = test_key("mig-old-b");
    let new_a = test_key("mig-new-a");
    let new_b = test_key("mig-new-b");
    let dead = test_key("mig-dead");
    let _g1 = KeyGuard(old_a.clone());
    let _g2 = KeyGuard(old_b.clone());
    let _g3 = KeyGuard(new_a.clone());
    let _g4 = KeyGuard(new_b.clone());
    let _g5 = KeyGuard(dead.clone());

    store_secret(old_a.clone(), "vaerdi-a".to_string()).expect("seed a");
    store_secret(old_b.clone(), "vaerdi-b".to_string()).expect("seed b");
    store_secret(dead.clone(), "doed".to_string()).expect("seed dead");

    talminal_canvas_lib::secrets::migrate_key_slots(
        &[
            (old_a.as_str(), new_a.as_str()),
            (old_b.as_str(), new_b.as_str()),
        ],
        &[dead.as_str()],
    )
    .expect("migration");

    assert_eq!(
        load_secret(new_a).expect("load new_a"),
        Some("vaerdi-a".to_string())
    );
    assert_eq!(
        load_secret(new_b).expect("load new_b"),
        Some("vaerdi-b".to_string())
    );
    assert_eq!(
        load_secret(old_a).expect("load old_a"),
        None,
        "gammel slot skal vaere ryddet"
    );
    assert_eq!(
        load_secret(dead).expect("load dead"),
        None,
        "doed slot skal vaere slettet"
    );
}

#[test]
fn migration_is_idempotent() {
    let old = test_key("mig-idem-old");
    let new = test_key("mig-idem-new");
    let _g1 = KeyGuard(old.clone());
    let _g2 = KeyGuard(new.clone());

    store_secret(old.clone(), "vaerdi".to_string()).expect("seed");
    let moves = [(old.as_str(), new.as_str())];
    talminal_canvas_lib::secrets::migrate_key_slots(&moves, &[]).expect("first");
    talminal_canvas_lib::secrets::migrate_key_slots(&moves, &[]).expect("second");

    assert_eq!(
        load_secret(new).expect("load new"),
        Some("vaerdi".to_string())
    );
}

#[test]
fn migration_does_not_overwrite_an_existing_new_slot() {
    let old = test_key("mig-keep-old");
    let new = test_key("mig-keep-new");
    let _g1 = KeyGuard(old.clone());
    let _g2 = KeyGuard(new.clone());

    store_secret(old.clone(), "gammel".to_string()).expect("seed old");
    store_secret(new.clone(), "allerede-sat".to_string()).expect("seed new");

    talminal_canvas_lib::secrets::migrate_key_slots(&[(old.as_str(), new.as_str())], &[])
        .expect("migration");

    assert_eq!(
        load_secret(new).expect("load new"),
        Some("allerede-sat".to_string()),
        "en allerede sat ny slot maa aldrig overskrives"
    );
    assert_eq!(load_secret(old).expect("load old"), None);
}

#[test]
fn migration_without_source_slots_is_a_noop() {
    let old = test_key("mig-noop-old");
    let new = test_key("mig-noop-new");
    let dead = test_key("mig-noop-dead");
    let _g1 = KeyGuard(new.clone());

    talminal_canvas_lib::secrets::migrate_key_slots(
        &[(old.as_str(), new.as_str())],
        &[dead.as_str()],
    )
    .expect("tom migration skal lykkes");

    assert_eq!(load_secret(new).expect("load new"), None);
}
