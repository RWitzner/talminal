// Task 14 — keyring-secrets' kontrakt, koert mod T1's in-memory-seam.
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
// T1: filen ramte indtil nu brugerens AEGTE Windows Credential Manager. Én
// koersel efterlod fem nye poster, og 311 `talminal-test-*`-poster havde
// ophobet sig. Under `test-seams` — som er slaaet til i HELE `cargo test` via
// self-dev-dependency'en — gaar store/load/delete nu til in-memory-storen i
// `secrets::store`, saa suiten kan koere paa en maskine helt uden en
// Credential Manager. Den aegte keyring naas kun af tests/keyring_smoke.rs,
// bag featuren `keyring-smoke` og under service "Talminal-smoke".
//
// Noeglenavnene forbliver unikke pr. koersel og pr. test: in-memory-storen er
// proces-global, saa parallelle tests i samme binaer deler den ene map. Og de
// forbliver test-specifikke — ALDRIG produktionsnavnene "stt_api_key"/
// "router_api_key" — saa en test aldrig kan komme til at maale eller
// overskrive en rigtig slot, uanset hvilken backend seamet peger paa.
//
// T2: teardown-guarden `KeyGuard` slugte sin fejl i et `let _ = delete_secret(
// ...)`. Den rapporterer nu, og tre tests maaler den paa EFFEKTEN i stedet for
// paa kaldet: at noeglen faktisk er vaek efter scopet, at guarden raaber naar
// sletningen fejler, og at den ikke maskerer en anden panik. Se doc-kommentaren
// ved `KeyGuard` for hvorfor en `Err` fra en idempotent `delete_secret` er en
// aegte fejl og ikke "noeglen var allerede slettet".

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use talminal_canvas_lib::secrets::{delete_secret, load_secret, store, store_secret};

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
/// panikker undervejs.
///
/// Fejl blev indtil T2 slugt i et `let _ = delete_secret(...)` med
/// begrundelsen "noeglen kan allerede vaere slettet". Den begrundelse er
/// forkert: `delete_secret` ER idempotent og svarer `Ok(())` paa en ukendt
/// noegle — det er en testet del af kontrakten (`delete_then_load_is_ok_none`).
/// En `Err` herfra betyder derfor ALDRIG "allerede slettet"; den betyder at
/// oprydningen ikke skete. Efter T1 rydder guarden ganske vist kun i
/// in-memory-storen, men den er stadig kontrakten "en test efterlader ikke
/// sine noegler" — og nu den eneste udgave af den der kan haandhaeves
/// deterministisk.
///
/// Panikken er vaernet af `std::thread::panicking()`, af samme grund som
/// `SmokeGuard` i tests/keyring_smoke.rs: paniker vi i en `Drop` der koerer
/// under unwinding fra en anden panik, kalder Rust `abort()`, og saa doer hele
/// binaeren uden at printe den OPRINDELIGE fejl. Teardown'en ville maskere
/// praecis den fejl den skal hjaelpe med at finde. I den gren baerer
/// `eprintln!` noeglenavnet ud i stedet, hvor panikken ikke kan bruges.
struct KeyGuard(String);

impl Drop for KeyGuard {
    fn drop(&mut self) {
        if let Err(e) = delete_secret(self.0.clone()) {
            if std::thread::panicking() {
                eprintln!("teardown af '{}' fejlede under unwind: {e}", self.0);
            } else {
                panic!("teardown af '{}' fejlede: {e}", self.0);
            }
        }
    }
}

/// Panik-payloaden som tekst. `catch_unwind` giver et `Box<dyn Any>`, og
/// `panic!` med hhv. literal og formatstreng lander som `&str` og `String` —
/// begge skal kunne laeses, ellers kan en assertion paa beskeden ikke skelne
/// den AEGTE fejl fra teardown'ens.
fn panic_besked(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "<ikke-tekstlig panik-payload>".to_string())
}

/// Beviset for at seamet faktisk er den vej de oevrige tests koerer: den
/// offentlige API skriver i IN-MEMORY-storen, ikke i OS'ets credential-store.
/// Fejler denne, er hele filens ikke-invasivitet en paastand uden daekning.
#[test]
fn den_offentlige_api_skriver_i_in_memory_storen() {
    let key = test_key("seam");
    let _guard = KeyGuard(key.clone());

    assert!(
        !store::contains_key_for_test(&key),
        "en frisk noegle maa ikke findes i forvejen"
    );
    store_secret(key.clone(), "kun-i-hukommelsen".to_string()).expect("store_secret");
    assert!(
        store::contains_key_for_test(&key),
        "store_secret skal ramme in-memory-storen — ikke Credential Manager"
    );

    delete_secret(key.clone()).expect("delete_secret");
    assert!(
        !store::contains_key_for_test(&key),
        "delete_secret skal fjerne noeglen fra in-memory-storen"
    );
}

/// Teardown-guarden maales paa sin EFFEKT, ikke paa at den blev kaldt.
///
/// Hver af filens oevrige tests hviler paa `KeyGuard` — men ingen af dem ville
/// blive roed hvis `Drop`-kroppen var tom: de skriver kun noegler ingen anden
/// test laeser. En invariant der udelukkende bekraeftes negativt (ingen test
/// klager) er ikke daekket. Derfor scopet nedenfor: guarden faar lov at doe,
/// og FOERST derefter spoerges storen om noeglen faktisk er vaek.
#[test]
fn key_guard_rydder_noeglen_ved_scope_exit() {
    let key = test_key("teardown-effekt");
    {
        let _guard = KeyGuard(key.clone());
        store_secret(key.clone(), "vaek-naar-scopet-slutter".to_string()).expect("store_secret");
        assert!(
            store::contains_key_for_test(&key),
            "seed'en skal ligge i storen mens guarden lever — ellers maaler \
             assertionen efter scopet ingenting"
        );
    }
    assert!(
        !store::contains_key_for_test(&key),
        "KeyGuard::drop skal have fjernet '{key}' fra storen ved scope-exit"
    );
}

/// Og guarden skal RAABE naar sletningen fejler — ellers er rapporteringen
/// selv en paastand uden daekning.
///
/// Stimulus er en blank noegle: `normalized_key` er faelles for begge backends
/// og afviser den, saa `delete_secret` returnerer `Err` uden bivirkninger. Det
/// er den ENESTE deterministiske fejlvej i in-memory-storen — en HashMap-remove
/// kan ikke fejle — og den kraever ingen ny affordance i produktionskoden.
/// Guarden skelner alligevel ikke mellem aarsager; den forgrener paa `Err`, saa
/// enhver `Err` er et trofast stimulus for den gren der testes her.
#[test]
fn key_guard_raaber_naar_sletningen_fejler() {
    let payload = std::panic::catch_unwind(|| {
        let _guard = KeyGuard(String::new());
    })
    .expect_err("en fejlet teardown maa IKKE passere tavst");

    let besked = panic_besked(&*payload);
    assert!(
        besked.contains("teardown af '' fejlede: secret key must be non-empty"),
        "panikken skal naevne baade noeglen og den underliggende fejl — men var: {besked}"
    );
}

/// ...men den maa ALDRIG raabe oven i en anden panik.
///
/// Paniker en `Drop` mens traaden allerede unwinder, kalder Rust `abort()`:
/// binaeren doer uden at printe den oprindelige fejl, og man staar med et
/// diagnoseloest abort i praecis det tilfaelde hvor man havde brug for
/// diagnosen. At denne test overhovedet naar sin assertion — i stedet for at
/// tage hele test-binaeren med sig — er halvdelen af beviset; den anden
/// halvdel er at payloaden er testens EGEN fejl og ikke teardown'ens.
#[test]
fn key_guard_maskerer_ikke_den_aegte_fejl() {
    let payload = std::panic::catch_unwind(|| {
        let _guard = KeyGuard(String::new());
        panic!("DEN AEGTE FEJL");
    })
    .expect_err("den oprindelige panik skal naa ud");

    let besked = panic_besked(&*payload);
    assert_eq!(
        besked, "DEN AEGTE FEJL",
        "teardown-fejlen overskrev testens egen panik — saa er den oprindelige \
         diagnose tabt"
    );
}

/// En mock der er MILDERE end det den erstatter, skjuler fejl i stedet for at
/// finde dem. Windows afviser en credential-blob over 2560 bytes UTF-16
/// (keyring-3.6.3/src/windows.rs:224), saa in-memory-storen skal afvise
/// praecis dér — ellers er en test der gemmer en ~2 kB gateway-token groen
/// her og roed i den byggede app.
#[test]
fn for_lang_vaerdi_afvises_som_i_credential_manager() {
    let key = test_key("toolong");
    let _guard = KeyGuard(key.clone());

    // 1280 UTF-16-enheder = praecis loftet; én mere er over.
    let paa_graensen = "a".repeat(1280);
    store_secret(key.clone(), paa_graensen).expect("en vaerdi PAA loftet skal accepteres");

    let over_graensen = "a".repeat(1281);
    let err = store_secret(key.clone(), over_graensen)
        .expect_err("en vaerdi over loftet skal afvises som i Credential Manager");
    assert!(
        err.contains("longer than platform limit of 2560"),
        "fejlen skal vaere keyrings egen ordlyd, saa en assertion holder mod \
         BEGGE backends — men var: {err}"
    );

    // Ikke-ASCII taeller i UTF-16-enheder, ikke i bytes: 'æ' er én enhed (2 B),
    // saa 1280 af dem rammer ogsaa praecis loftet.
    store_secret(key.clone(), "æ".repeat(1280)).expect("1280 UTF-16-enheder er paa loftet");
    let err = store_secret(key, "æ".repeat(1281)).expect_err("1281 enheder er over");
    assert!(err.contains("longer than platform limit of 2560"));
}

/// Noeglevalideringen er faelles for begge backends (`secrets::normalized_key`),
/// saa denne test beviser ogsaa produktionens kontrakt: en tom eller kun-blanke
/// noegle er en fejl paa alle tre veje, ikke en tavs no-op.
#[test]
fn tom_noegle_afvises_paa_alle_tre_veje() {
    for blank in ["", "   ", "\t\n"] {
        let store_err = store_secret(blank.to_string(), "v".to_string())
            .expect_err("tom noegle maa ikke kunne gemmes");
        assert!(
            store_err.contains("must be non-empty"),
            "uventet fejl fra store_secret: {store_err}"
        );
        let load_err =
            load_secret(blank.to_string()).expect_err("tom noegle maa ikke kunne loades");
        assert!(
            load_err.contains("must be non-empty"),
            "uventet fejl fra load_secret: {load_err}"
        );
        let delete_err =
            delete_secret(blank.to_string()).expect_err("tom noegle maa ikke kunne slettes");
        assert!(
            delete_err.contains("must be non-empty"),
            "uventet fejl fra delete_secret: {delete_err}"
        );
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
