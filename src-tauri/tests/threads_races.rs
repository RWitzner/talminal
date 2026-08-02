//! Spec §7's syv race-cases. De hoerer ikke til én task, fordi de krydser hele
//! motoren — og en test der kalder to funktioner efter hinanden beviser ikke et
//! race. Case 1-4 bruger derfor rigtige `std::thread`s og en `Barrier`, saa de
//! konkurrerende kald slippes loes i samme oejeblik. Case 5-7 er
//! raekkefoelge-cases uden samtidig formulering (backlog-batching, genbrugt
//! navn, crash mellem to trin) og koeres sekventielt med vilje.
//!
//! Ottende race — svar mod sweeperens snapshot-vindue — bor i
//! `threads_sweep.rs`, hvor fake-uret der driver fristerne allerede staar. Den
//! rammer vinduet gennem en test-seam frem for en `Barrier`, af praecis den
//! grund `HeadStart` herunder dokumenterer.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use talminal_canvas_lib::threads::dispatch;
use talminal_canvas_lib::threads::pair::{self, SpawnedCard, Spawner};
use talminal_canvas_lib::threads::policy::AcceptsFromView;
use talminal_canvas_lib::threads::{
    self, FromKind, Intent, PostRequest, TerminalReason, ThreadState,
};

struct RealishClock;
impl dispatch::Clock for RealishClock {
    fn now_ms(&self) -> u64 {
        0
    }
}
struct Silent;
impl dispatch::Notifier for Silent {
    fn is_busy(&self, _card: &str) -> bool {
        false
    }
    fn write_notice(&self, _card: &str, _text: &str) -> Result<(), String> {
        Ok(())
    }
}

fn fresh() {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    threads::reset_activity_probe_for_test();
    dispatch::reset_for_test();
    dispatch::set_seams(Arc::new(RealishClock), Arc::new(Silent));
}

fn agent_post(thread: &str, from: &str, intent: Intent) -> Result<threads::PostAccepted, String> {
    threads::post(PostRequest {
        thread: thread.into(),
        from_card: from.into(),
        from_kind: FromKind::Agent,
        intent,
        text: "x".into(),
    })
}

/// Seq'erne er tildelt under traadlaasen. Er de ikke 1..=n uden huller eller
/// dubletter, har to samtidige skrivninger set den samme laengde.
fn seqs_are_contiguous(t: &threads::Thread) -> bool {
    t.messages
        .iter()
        .enumerate()
        .all(|(i, m)| m.seq == i as u64 + 1)
}

/// Hvilken af de to konkurrenter der faar et forspring i denne omgang.
///
/// En BAR `Barrier` er ikke nok, og det er maalt: `post()` tager traadlaasen,
/// slipper den, laeser politikken og tager den igen, mens `close_thread()` gaar
/// lige i laasen. Slippes de loes praecis samtidig, vinder lukkeren 49-50 ud af
/// 50 gange — halvdelen af raceten forbliver altsaa uafproevet, og en test der
/// kun naar ét udfald beviser praecis dét ene udfald. Derfor koeres begge
/// raekkefoelger med et eksplicit forspring OVEN PAA de ustyrede omgange, og
/// testen slutter med at paastaa at begge udfald faktisk blev set.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HeadStart {
    /// Ingen: ren, ustyret kappestrid om laasen.
    Neither,
    /// Foerste konkurrent (svar hhv. post) slippes foerst.
    First,
    /// Anden konkurrent (lukkeren) slippes foerst.
    Second,
}

impl HeadStart {
    fn for_round(i: usize) -> Self {
        match i % 3 {
            0 => HeadStart::Neither,
            1 => HeadStart::First,
            _ => HeadStart::Second,
        }
    }

    /// Kaldes lige efter `barrier.wait()` i hver af de to traade.
    fn hold_back(self, i_am_first: bool) {
        let yield_me = match self {
            HeadStart::Neither => false,
            HeadStart::First => !i_am_first,
            HeadStart::Second => i_am_first,
        };
        if yield_me {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

/// 1. answer-mod-timeout: praecis EEN transition vinder delegeringen.
#[test]
fn exactly_one_transition_wins_the_pending_delegation() {
    let _g = common::serial();
    let _home = common::temp_home();
    let (mut answer_won, mut answer_lost) = (0usize, 0usize);
    for round in 0..50 {
        fresh();
        threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
        agent_post("t1", "card-1", Intent::Delegation).unwrap();

        let head_start = HeadStart::for_round(round);
        let barrier = Arc::new(Barrier::new(2));
        let b1 = barrier.clone();
        let answer = std::thread::spawn(move || {
            b1.wait();
            head_start.hold_back(true);
            agent_post("t1", "card-2", Intent::Answer).is_ok()
        });
        let b2 = barrier.clone();
        let timeout = std::thread::spawn(move || {
            b2.wait();
            head_start.hold_back(false);
            threads::close_thread(
                "t1",
                TerminalReason::AbsoluteTimeout { peer_active: true },
                None,
            )
        });
        let answered = answer
            .join()
            .expect("svar-traaden maa ikke panice eller haenge");
        let timed_out = timeout
            .join()
            .expect("timeout-traaden maa ikke panice eller haenge");

        let t = threads::get("t1").unwrap();
        assert!(t.pending.is_none(), "delegeringen skal vaere afgjort");
        // Praecis én af de to vandt. Vandt timeouten, er traaden lukket og
        // svaret afvist; vandt svaret, er traaden aaben og timeouten tabte.
        if timed_out {
            assert!(!answered || t.state == ThreadState::Closed);
        } else {
            assert!(answered, "en af dem SKAL have vundet");
        }
        let systems = t
            .messages
            .iter()
            .filter(|m| m.from_kind == FromKind::System)
            .count();
        assert_eq!(
            systems, 1,
            "aldrig to terminalbeskeder for samme delegering"
        );
        // Vandt svaret kaploebet, ligger det i historikken; tabte det, findes
        // det slet ikke. Et halvt accepteret svar er ikke et tredje udfald.
        let answers = t
            .messages
            .iter()
            .filter(|m| m.intent == Intent::Answer)
            .count();
        assert_eq!(
            usize::from(answered),
            answers,
            "et afvist svar maa ikke staa i traaden"
        );
        assert!(
            seqs_are_contiguous(&t),
            "to samtidige skrivninger delte en seq"
        );

        if answered {
            answer_won += 1
        } else {
            answer_lost += 1
        }
    }
    // Selve daekningen er en paastand: uden den ville testen kunne "bestaa"
    // med 50 identiske udfald og aldrig have roert den anden gren.
    assert!(
        answer_won > 0,
        "svaret vandt aldrig — kun én raekkefoelge blev proevet"
    );
    assert!(
        answer_lost > 0,
        "svaret tabte aldrig — kun én raekkefoelge blev proevet"
    );
}

/// 2. luk-mod-post: ingen deadlock, og ingen besked i en lukket traad.
#[test]
fn closing_while_posting_neither_deadlocks_nor_leaks_a_message() {
    let _g = common::serial();
    let _home = common::temp_home();
    let (mut post_won, mut post_lost) = (0usize, 0usize);
    for round in 0..50 {
        fresh();
        threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
        let head_start = HeadStart::for_round(round);
        let barrier = Arc::new(Barrier::new(2));
        let b1 = barrier.clone();
        let poster = std::thread::spawn(move || {
            b1.wait();
            head_start.hold_back(true);
            agent_post("t1", "card-1", Intent::Sparring)
        });
        let b2 = barrier.clone();
        let closer = std::thread::spawn(move || {
            b2.wait();
            head_start.hold_back(false);
            threads::close_thread("t1", TerminalReason::OwnerStopped, None)
        });
        let posted = poster
            .join()
            .expect("post-traaden maa ikke panice eller haenge");
        closer
            .join()
            .expect("luk-traaden maa ikke panice eller haenge");

        let t = threads::get("t1").unwrap();
        assert_eq!(t.state, ThreadState::Closed);
        assert!(
            seqs_are_contiguous(&t),
            "to samtidige skrivninger delte en seq"
        );
        if posted.is_ok() {
            // Beskeden kom foer lukningen: den er i historikken, men ingen
            // inbox eller wake maa have overlevet oprydningen bortset fra
            // terminalbeskedens egen.
            let terminal_seq = t.messages.last().unwrap().seq;
            for queue in t.inbox.values() {
                assert!(
                    queue.iter().all(|s| *s == terminal_seq),
                    "forældet inbox-post overlevede"
                );
            }
            post_won += 1;
        } else {
            // Lukningen kom foerst: beskeden findes ikke, hverken i historikken
            // eller i nogen koe.
            assert!(
                t.messages.iter().all(|m| m.from_kind == FromKind::System),
                "en afvist post lakkede ind i den lukkede traad"
            );
            post_lost += 1;
        }
    }
    assert!(
        post_won > 0,
        "posten vandt aldrig — kun én raekkefoelge blev proevet"
    );
    assert!(
        post_lost > 0,
        "posten tabte aldrig — kun én raekkefoelge blev proevet"
    );
}

/// 3. tre samtidige card_pair fra SAMME foraelder: kun to passerer loftet.
#[test]
fn three_concurrent_pairings_respect_the_child_cap() {
    let _g = common::serial();
    let _home = common::temp_home();
    #[derive(Default)]
    struct CountingSpawner {
        spawned: AtomicUsize,
        closed: AtomicUsize,
    }
    impl Spawner for CountingSpawner {
        fn spawn(&self, agent: &str, _cwd: &str) -> Result<SpawnedCard, String> {
            let n = self.spawned.fetch_add(1, Ordering::SeqCst);
            Ok(SpawnedCard {
                name: format!("partner-{n}"),
                agent: agent.into(),
            })
        }
        fn verify_running_with_mcp(&self, _card: &str) -> Result<(), String> {
            Ok(())
        }
        fn close(&self, _card: &str) {
            self.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    fresh();
    pair::reset_for_test();
    let spawner = Arc::new(CountingSpawner::default());
    pair::set_spawner(spawner.clone());

    let barrier = Arc::new(Barrier::new(3));
    let results: Vec<bool> = (0..3)
        .map(|i| {
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                pair::card_pair("card-1", "codex", &format!("p{i}"), "hej").is_ok()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().expect("en parrings-traad panicede eller haengte"))
        .collect();

    assert_eq!(
        results.iter().filter(|ok| **ok).count(),
        pair::MAX_CHILDREN as usize
    );
    // Den tredje maa ikke efterlade et spawnet kort: slottet reserveres FOER
    // noget synligt oprettes, saa taberen naar aldrig at spawne.
    assert_eq!(
        spawner.spawned.load(Ordering::SeqCst) - spawner.closed.load(Ordering::SeqCst),
        pair::MAX_CHILDREN as usize,
        "ingen forældreløse partnere"
    );
    for card in talminal_canvas_lib::registry::list_cards() {
        if card.kind == "chat" {
            talminal_canvas_lib::registry::close_card(card.name).ok();
        }
    }
}

/// 4. tabt inbox-svar: samme batch genleveres med identiske seq'er.
#[test]
fn a_lost_inbox_response_redelivers_the_identical_batch() {
    let _g = common::serial();
    let _home = common::temp_home();
    fresh();
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    for _ in 0..3 {
        agent_post("t1", "card-1", Intent::Sparring).unwrap();
    }
    let barrier = Arc::new(Barrier::new(4));
    let batches: Vec<Vec<u64>> = (0..4)
        .map(|_| {
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                threads::inbox_take("t1", "card-2", None)
                    .unwrap()
                    .messages
                    .iter()
                    .map(|m| m.seq)
                    .collect::<Vec<u64>>()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().expect("en hente-traad panicede eller haengte"))
        .collect();
    assert_eq!(
        batches[0],
        vec![1, 2, 3],
        "batchen er de tre koede beskeder"
    );
    assert!(
        batches.windows(2).all(|w| w[0] == w[1]),
        "uden ack er hentningen ren: {batches:?}"
    );
}

/// 5. backlog over fem: has_more, og ack af batch 1 afsloerer batch 2.
#[test]
fn a_backlog_over_five_is_revealed_batch_by_batch() {
    let _g = common::serial();
    let _home = common::temp_home();
    fresh();
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    for _ in 0..12 {
        agent_post("t1", "card-1", Intent::Sparring).unwrap();
    }
    let mut seen: Vec<u64> = Vec::new();
    let mut ack = None;
    loop {
        let batch = threads::inbox_take("t1", "card-2", ack).unwrap();
        if batch.messages.is_empty() {
            assert!(!batch.has_more, "en tom batch kan ikke have mere bag sig");
            break;
        }
        assert!(batch.messages.len() <= threads::INBOX_BATCH_MAX);
        assert_eq!(
            batch.has_more,
            seen.len() + batch.messages.len() < 12,
            "has_more skal spejle den faktiske rest"
        );
        seen.extend(batch.messages.iter().map(|m| m.seq));
        ack = Some(batch.batch_id);
    }
    assert_eq!(
        seen.len(),
        12,
        "alle beskeder blev leveret praecis én gang: {seen:?}"
    );
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "og i raekkefoelge");
}

/// 6. genbrugt kortnavn: et lukket card-2 og et nyt card-2 deler ikke traad.
#[test]
fn a_reused_card_name_cannot_slip_into_an_old_thread() {
    let _g = common::serial();
    let _home = common::temp_home();
    fresh();
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    agent_post("t1", "card-1", Intent::Delegation).unwrap();

    // card-2 doer. T11's lukning er praemissen for at navnet kan genbruges.
    threads::on_card_gone("card-2", None);
    assert_eq!(threads::get("t1").unwrap().state, ThreadState::Closed);

    // Et NYT kort med samme navn maa ikke kunne skrive i den gamle traad.
    let err = agent_post("t1", "card-2", Intent::Sparring).unwrap_err();
    assert!(err.contains("closed"), "{err}");
    assert!(
        threads::threads_with_member("card-2").is_empty(),
        "en lukket traad maa ikke laengere regnes som medlemskab"
    );
}

/// 7. crash mellem PairTxn-trin: opstarts-passet rydder det halve resultat.
#[test]
fn a_crash_between_pair_steps_is_terminalized_on_the_next_startup() {
    let _g = common::serial();
    let home = common::temp_home();
    fresh();
    // Simulér: traad + delegering naaede disken, men appen doede foer et udfald.
    threads::create_thread("t1", "p", vec!["card-1".into(), "card-2".into()]).unwrap();
    agent_post("t1", "card-1", Intent::Delegation).unwrap();
    // Ny proces: kun arkivet overlever (workspace.json genskaber ingen kort).
    threads::reset_for_test();
    assert_eq!(
        threads::archive::terminalize_awaiting_on_startup().unwrap(),
        1
    );

    let body = std::fs::read_to_string(home.path().join("threads").join("t1.jsonl")).unwrap();
    let last: serde_json::Value = serde_json::from_str(body.lines().last().unwrap()).unwrap();
    assert_eq!(last["terminal_reason"], "restart_abort");
}
