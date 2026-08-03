// Task 8 — Rust-side output-gating pr. kort (spec-FUND 2: alt-screen har
// ingen historik at miste, saa skjulte korts chunks DROPPES — de bufres ikke).
//
// Testbarhed-reglen: suppressions-beslutningen er den DELTE lib-funktion
// registry::gated_emit — main.rs' reader-emit-closure og denne test wrapper
// deres emit i PRAECIS samme funktion, saa testen rammer produktionslogikken
// og ikke en kopi. Flaget bor i CardRuntime (registry.rs, Arc<AtomicBool>).
//
// Kritisk invariant (plan Task 8, bindende): visible=false gater KUN emitten.
// Pty'en LAESES stadig (backpressure maa aldrig ramme child'en) — bevist
// dobbelt: bytes_read stiger under det gatede vindue OG child'en lever.
//
// Synkrone #[test] (ingen tokio); parallel-robuste asserts pr. eget kort.
// Exit observeres via try_exit_status — ALDRIG via reader-EOF (FUND 11).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use talminal_canvas_lib::pty::{PtyHost, PtySpawn};
use talminal_canvas_lib::registry;

mod common;
use common::cmd_exe;

/// Poll indtil cond() eller timeout; returnerer sidste cond-vaerdi.
fn wait_until(timeout: Duration, cond: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    cond()
}

// ---------- (a)+(b) gating-semantikken mod et flood-child ----------

#[test]
fn hidden_card_suppresses_emits_but_reader_keeps_draining_and_child_lives() {
    let dir = tempfile::tempdir().unwrap();
    // Flood-child: uendelig cmd-loekke (step 0 naar aldrig 1) — kontinuerligt
    // output, saa "laesningen fortsatte" kan maales inde i det gatede vindue.
    let info = registry::create_card(
        dir.path().display().to_string(),
        "claude".to_string(),
        Some(format!(
            "{} /c for /L %i in (0,0,1) do @echo FLOOD",
            cmd_exe()
        )),
    )
    .expect("create_card with flood command");

    let handle = registry::card_handle(&info.name).expect("card_handle");
    let (command, cwd, visible) = {
        let card = handle.lock().unwrap();
        let term = card.terminal().expect("terminal card");
        (
            term.config.command.clone(),
            term.config.cwd.clone(),
            Arc::clone(&term.visible),
        )
    };

    // Emit-taelleren spiller main.rs' "pty-output"-emit; beslutningen om at
    // kalde den er registry::gated_emit — samme funktion som i spawn_into.
    let events = Arc::new(AtomicUsize::new(0));
    let ev = Arc::clone(&events);
    let host = Arc::new(
        PtyHost::spawn(
            PtySpawn {
                cwd,
                command,
                cols: 80,
                rows: 24,
                env_deny_prefixes: vec![],
                env_deny_exact: vec![],
                extra_env: vec![],
            },
            registry::gated_emit(visible, move |_bytes: &[u8]| {
                ev.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .expect("spawn flood child"),
    );
    {
        let mut card = handle.lock().unwrap();
        card.terminal_mut().expect("terminal card").pty = Some(Arc::clone(&host));
    }

    // Fase 0: kort er synlige som default — events skal flyde.
    assert!(
        wait_until(Duration::from_secs(10), || events.load(Ordering::SeqCst)
            > 0),
        "default-synligt kort skal emitte pty-output-chunks"
    );

    // (a) visible=false ⇒ 0 NYE emits over 2 s — men laesningen fortsaetter.
    registry::set_card_visible(info.name.clone(), false).expect("set_card_visible(false)");
    // Kort settle: en chunk laest FOER flippet kan vaere paa vej gennem
    // closuren — baseline tages efter roen har lagt sig.
    thread::sleep(Duration::from_millis(250));
    let events_baseline = events.load(Ordering::SeqCst);
    let bytes_baseline = host.bytes_read();
    thread::sleep(Duration::from_secs(2));
    assert_eq!(
        events.load(Ordering::SeqCst),
        events_baseline,
        "skjult kort maa ikke emitte pty-output (chunks droppes — FUND 2)"
    );
    assert!(
        host.bytes_read() > bytes_baseline,
        "reader-traaden skal FORTSAT draene pty'en mens kortet er skjult \
         (backpressure maa aldrig ramme child'en)"
    );
    assert!(
        host.try_exit_status().is_none(),
        "flood-child'en skal stadig leve efter 2 s gated laesning"
    );

    // (b) visible=true ⇒ emit genoptages.
    registry::set_card_visible(info.name.clone(), true).expect("set_card_visible(true)");
    let resumed_from = events.load(Ordering::SeqCst);
    assert!(
        wait_until(Duration::from_secs(10), || {
            events.load(Ordering::SeqCst) > resumed_from
        }),
        "events skal flyde igen efter visible=true"
    );

    // Cleanup: close_card river pty'en ned (normativ teardown).
    registry::close_card(info.name).expect("close_card cleanup");
}

// ---------- fejlflade ----------

#[test]
fn set_card_visible_on_unknown_card_is_descriptive_error() {
    let err = registry::set_card_visible("card-808808808".to_string(), false)
        .expect_err("unknown card must be an error");
    assert!(err.contains("unknown card"), "fik: {err}");
    assert!(
        err.contains("card-808808808"),
        "fejlen skal naevne navnet, fik: {err}"
    );
}

// ---------- flaget er uafhaengigt af running-state ----------

#[test]
fn set_card_visible_works_for_non_running_card() {
    // LOD-laget (Task 10) kalder set_card_visible for kort i viewporten
    // uanset om de er spawnet — flaget bor paa CardRuntime, ikke paa pty'en.
    let dir = tempfile::tempdir().unwrap();
    let info = registry::create_card(dir.path().display().to_string(), "claude".to_string(), None)
        .expect("create_card");
    registry::set_card_visible(info.name.clone(), false).expect("hide non-running card");
    registry::set_card_visible(info.name.clone(), true).expect("show non-running card");
    registry::close_card(info.name).expect("close cleanup");
}
