//! Timeout-trappen. Backstoppen er en selvstaendig vej for delegeringer hvis
//! notits aldrig blev leveret og derfor aldrig fik normale deadlines.

use super::{dispatch, TerminalReason, ThreadState};

struct Candidate {
    id: String,
    /// Delegeringens `seq`. Snapshottet er taget under laasen, men lukkes der
    /// foerst efter at laasen er sluppet — dette felt er kvitteringen paa at
    /// det stadig er SAMME delegering vi lukker (se `close_thread_if`).
    request_seq: u64,
    assignee: String,
    request_ts_ms: u64,
    delivered_at_ms: Option<u64>,
    idle_deadline_ms: Option<u64>,
    absolute_deadline_ms: Option<u64>,
}

pub fn sweep() {
    let now = dispatch::now_ms();
    let candidates = {
        let map = super::lock_threads();
        map.values()
            .filter(|thread| thread.state == ThreadState::Awaiting)
            .filter_map(|thread| {
                let pending = thread.pending.as_ref()?;
                Some(Candidate {
                    id: thread.id.clone(),
                    request_seq: pending.request_seq,
                    assignee: pending.assignee.clone(),
                    request_ts_ms: pending.request_ts_ms,
                    delivered_at_ms: pending.delivered_at_ms,
                    idle_deadline_ms: pending.idle_deadline_ms,
                    absolute_deadline_ms: pending.absolute_deadline_ms,
                })
            })
            .collect::<Vec<_>>()
    };

    // Laasen er sluppet her, og `peer_active` tager selv tre andre laase: fra og
    // med dette punkt er hver kandidat kun en HYPOTESE om traadens tilstand.
    #[cfg(feature = "test-seams")]
    run_pre_close_hook();

    for candidate in candidates {
        match candidate.absolute_deadline_ms {
            Some(absolute) => {
                let peer_active = super::peer_active(&candidate.assignee);
                if now > absolute {
                    super::close_thread_if(
                        &candidate.id,
                        TerminalReason::AbsoluteTimeout { peer_active },
                        None,
                        candidate.request_seq,
                    );
                } else if candidate.idle_deadline_ms.is_some_and(|idle| now > idle) && !peer_active
                {
                    super::close_thread_if(
                        &candidate.id,
                        TerminalReason::IdleTimeout { peer_active },
                        None,
                        candidate.request_seq,
                    );
                }
            }
            None => {
                let anchor = candidate.delivered_at_ms.unwrap_or(candidate.request_ts_ms);
                if now.saturating_sub(anchor) > dispatch::BACKSTOP_MS {
                    super::close_thread_if(
                        &candidate.id,
                        TerminalReason::BackstopCleanup,
                        None,
                        candidate.request_seq,
                    );
                }
            }
        }
    }
}

/// Test-seam der koeres PRAECIS i vinduet mellem snapshottet og lukningen.
///
/// Vinduet kan ikke rammes paalideligt med to traade og en `Barrier`: den
/// maaling er allerede gjort i dette repo (lukkeren vandt 149 ud af 150
/// omgange), saa en test der bare slipper to traade loes beviser det ene udfald
/// den tilfaeldigvis ramte. Hooken er ETGANGS inden for det sweep der tager
/// den; at den ikke laekker VIDERE til naeste test sikres af
/// `reset_pre_close_hook_for_test` i testfilens `setup()`.
#[cfg(feature = "test-seams")]
pub type PreCloseHook = Box<dyn FnOnce() + Send>;

#[cfg(feature = "test-seams")]
fn pre_close_hook() -> &'static std::sync::Mutex<Option<PreCloseHook>> {
    static HOOK: std::sync::OnceLock<std::sync::Mutex<Option<PreCloseHook>>> =
        std::sync::OnceLock::new();
    HOOK.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(feature = "test-seams")]
pub fn set_pre_close_hook_for_test(hook: PreCloseHook) {
    *pre_close_hook().lock().unwrap_or_else(|p| p.into_inner()) = Some(hook);
}

/// Modstykket til saetteren, kaldes fra testfilens `setup()`.
///
/// Etgangs-reglen gaelder kun DEN koersel der naar at tage hooken: saetter en
/// test en hook og fejler foer sweepet, staar den stadig og loeber i den naeste
/// tests sweep — samme proces, andet regnestykke. Derfor har den et reset som
/// alle andre proces-globale seams i modulet.
#[cfg(feature = "test-seams")]
pub fn reset_pre_close_hook_for_test() {
    *pre_close_hook().lock().unwrap_or_else(|p| p.into_inner()) = None;
}

#[cfg(feature = "test-seams")]
fn run_pre_close_hook() {
    // Laasen slippes FOER hooken koeres: den poster typisk i den traad
    // sweeperen er ved at kigge paa.
    let hook = pre_close_hook()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

/// Timeout-trappen er ren konstant-aritmetik og haandhæves derfor paa
/// kompile-tid i stedet for som runtime-assert (spec 4.5).
const _: () = assert!(
    dispatch::BACKSTOP_MS > dispatch::ABSOLUTE_MS && dispatch::ABSOLUTE_MS > dispatch::IDLE_MS,
    "timeout-trappen skal overholde backstop > absolut > idle (spec 4.5)"
);

pub fn assert_startup_invariants() {
    assert!(
        super::policy::is_wired(),
        "the accepts_from policy port is not wired: every agent-to-agent post \
         would silently fall back to human-only and the feature would be dead"
    );
}
