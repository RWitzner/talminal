// Danske testnavne bruger VERSALER som betoning (suite-konvention).
#![allow(non_snake_case)]

use talminal_canvas_lib::workspaces::protocol::{decide, Action, Inputs};
use talminal_canvas_lib::workspaces::{
    active_path, next_request, write_active, ActiveRequest, AttentionKind, RequestId,
    WorkspaceStatus,
};

const NU: &str = "2026-07-26T10:00:00.000Z";
const EFTER_DEADLINE: &str = "2026-07-26T10:00:20.000Z";

/// `any_visible` som navngivne konstanter — en bool mere i en positionsliste der
/// allerede har `target_alive` ville være en fælde ved næste redigering.
const NOGEN_ER_SYNLIG: bool = true;
const INGEN_ER_SYNLIG: bool = false;

fn id(issuer: &str, seq: u64) -> RequestId {
    RequestId {
        issuer: issuer.into(),
        seq,
    }
}

fn status(pid: u32, acked: Option<RequestId>, visible: bool) -> WorkspaceStatus {
    WorkspaceStatus {
        instance_id: format!("inst-{pid}"),
        pid,
        acked_request: acked,
        visible,
        cards: 0,
        running_cards: 0,
        attention: false,
        attention_kind: AttentionKind::None,
        updated_at: NU.into(),
    }
}

fn req(request: RequestId, slug: &str, deadline: &str) -> ActiveRequest {
    ActiveRequest {
        id: request,
        slug: slug.into(),
        launch_deadline: deadline.into(),
    }
}

/// Standard-scenarie: jeg er "a", synlig, og der er en request mod "b".
#[allow(clippy::too_many_arguments)]
fn inputs<'a>(
    me: &'a str,
    my_status: &'a WorkspaceStatus,
    active: Option<&'a ActiveRequest>,
    target_status: Option<&'a WorkspaceStatus>,
    now: &'a str,
    target_alive: bool,
    live_slugs: &'a [String],
    any_visible: bool,
) -> Inputs<'a> {
    Inputs {
        me,
        my_status,
        active,
        target_status,
        now,
        target_alive,
        live_slugs,
        any_visible,
    }
}

#[test]
fn target_der_er_skjult_afsloerer_sig() {
    let me = status(1, None, false);
    let r = req(id("ui", 5), "b", EFTER_DEADLINE);
    let live = vec!["b".to_string()];
    assert_eq!(
        decide(&inputs(
            "b",
            &me,
            Some(&r),
            None,
            NU,
            true,
            &live,
            INGEN_ER_SYNLIG
        )),
        Action::Reveal { request: r.clone() }
    );
}

#[test]
fn kilden_skjuler_sig_kun_ved_ack_med_praecis_samme_request_identitet() {
    let me = status(1, Some(id("ui", 4)), true);
    let r = req(id("ui", 5), "b", EFTER_DEADLINE);
    let live = vec!["a".to_string(), "b".to_string()];

    let venter = status(2, None, false);
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&venter),
            NU,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Nothing
    );

    // Stale visible:true fra et crash — bærer et FORÆLDET id.
    let stale = status(2, Some(id("ui", 4)), true);
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&stale),
            NU,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Nothing,
        "stale visible:true med gammelt id må aldrig tælle som kvittering"
    );

    // Samme seq, ANDEN udsteder — ABA-tilfældet. To processer kan udstede seq 5
    // uafhængigt; kun parret (issuer, seq) er entydigt.
    let aba = status(2, Some(id("cli", 5)), true);
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&aba),
            NU,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Nothing,
        "samme seq fra en anden udsteder er en ANDEN request"
    );

    let acked = status(2, Some(id("ui", 5)), true);
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&acked),
            NU,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Conceal
    );
}

#[test]
fn et_ack_uden_visible_er_ikke_en_kvittering() {
    // Spec §3.2 punkt 3 kræver BEGGE dele: "præcis `acked_request == n+1` OG
    // `visible == true`". `acked_request` ryddes aldrig — feltet bliver stående
    // efter at target selv har skjult sig igen — så id-matchet alene er bevis
    // for at target EN GANG har vist sig for denne request, ikke for at der står
    // et vindue på skærmen nu. Skjulte kilden sig mod dét, ville ingen være
    // fremme.
    let me = status(1, Some(id("ui", 4)), true);
    let r = req(id("ui", 5), "b", EFTER_DEADLINE);
    let live = vec!["a".to_string(), "b".to_string()];

    let acked_men_skjult = status(2, Some(id("ui", 5)), false);
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&acked_men_skjult),
            NU,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Nothing,
        "et target der ikke er fremme må ikke kunne få kilden til at skjule sig"
    );

    // POSITIV KONTROL: samme fixtur, kun `visible` vendt — så SKAL den samme
    // kode-vej conceale. Uden den ville `fn decide(_) -> Nothing` bestå.
    let acked_og_fremme = status(2, Some(id("ui", 5)), true);
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&acked_og_fremme),
            NU,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Conceal
    );
}

#[test]
fn et_ack_uden_visible_ruller_tilbage_efter_deadline() {
    // Modstykket til ovenstående: bliver target ved med at være tavs efter
    // fristen, må kilden ikke stå og vente for evigt på et vindue der aldrig
    // kommer frem. Den tager requesten tilbage — præcis som ved et ack der
    // aldrig kom.
    let me = status(1, Some(id("ui", 4)), true);
    let r = req(id("ui", 5), "b", NU);
    let live = vec!["a".to_string(), "b".to_string()];
    let acked_men_skjult = status(2, Some(id("ui", 5)), false);

    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&acked_men_skjult),
            EFTER_DEADLINE,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Rollback {
            failed_slug: "b".into(),
            observed: id("ui", 5)
        },
    );
}

#[test]
fn stoppet_workspace_med_gammel_status_fil_kan_stadig_startes() {
    // REGRESSION (plan-review P0-4): et stoppet workspace beholder sin status.json.
    // Rev 1 tolkede "status-fil findes + ingen .lock" som dødt target og rullede
    // tilbage FØR deadline — så et stoppet workspace kunne aldrig startes.
    // Kriteriet er nu "har acket DENNE request", ikke "har en fil".
    let me = status(1, Some(id("ui", 4)), true);
    let gammel = status(2, Some(id("ui", 2)), false); // fra en tidligere session
    let r = req(id("ui", 5), "b", EFTER_DEADLINE);
    let live = vec!["a".to_string()];

    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&gammel),
            NU,
            false,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Nothing,
        "en kold start må have hele sin deadline, også når en gammel status-fil ligger der"
    );

    // POSITIV KONTROL i samme test: uden den ville `fn decide(_) -> Nothing`
    // bestå. Præcis samme fixtur, kun uret er rykket forbi deadline — så SKAL
    // den samme kode-vej rulle tilbage. Asserten ovenfor måler dermed
    // deadline-vinduet, ikke bare fraværet af en handling.
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&gammel),
            "2026-07-26T10:00:21.000Z",
            false,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Rollback {
            failed_slug: "b".into(),
            observed: id("ui", 5)
        },
        "efter deadline skal den gamle status-fil stadig give rollback"
    );
}

#[test]
fn rollback_naar_deadline_er_overskredet_uden_ack() {
    let me = status(1, Some(id("ui", 4)), true);
    let r = req(id("ui", 5), "b", NU);
    let live = vec!["a".to_string()];
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            None,
            EFTER_DEADLINE,
            false,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Rollback {
            failed_slug: "b".into(),
            observed: id("ui", 5)
        }
    );
}

#[test]
fn sen_ack_efter_deadline_taeller_stadig() {
    let me = status(1, Some(id("ui", 4)), true);
    let sen = status(2, Some(id("ui", 5)), true);
    let r = req(id("ui", 5), "b", NU);
    let live = vec!["a".to_string(), "b".to_string()];
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&sen),
            EFTER_DEADLINE,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Conceal,
        "en target der nåede frem må ikke rulles tilbage"
    );
}

#[test]
fn target_doed_EFTER_ack_giver_rollback_ikke_conceal() {
    // REGRESSION (plan-review P0-1): rev 1 returnerede Conceal før liveness-checket,
    // så denne test ville fejle deterministisk mod sin egen implementering.
    let me = status(1, Some(id("ui", 4)), true);
    let doed = status(2, Some(id("ui", 5)), true);
    let r = req(id("ui", 5), "b", EFTER_DEADLINE);
    let live = vec!["a".to_string()];
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&doed),
            NU,
            false,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Rollback {
            failed_slug: "b".into(),
            observed: id("ui", 5)
        },
        "liveness skal afgøres FØR en kvittering accepteres"
    );
}

#[test]
fn laveste_levende_slug_overtager_naar_alle_er_skjulte() {
    // REGRESSION (plan-review P0-1, anden halvdel): dør target EFTER at kilden har
    // skjult sig, er ingen synlig og ingen driver noget. En skjult proces skal
    // kunne tage over — deterministisk, så to ikke gør det samtidigt.
    //
    // Fixturen modellerer sit eget navn. `target_status: None` (rev 1) var
    // fingeraftrykket fra en KOLD START der endnu ikke er nået frem — den tilstand
    // spec §3.3 udtrykkeligt forbyder at behandle som død — så testen asserterede
    // TakeOver i den forbudte tilstand og var grøn mod både den rigtige og den
    // forkerte implementation. Her HAR target acket og været synlig, og er derefter
    // død.
    let me = status(1, Some(id("ui", 4)), false);
    let r = req(id("ui", 5), "c", EFTER_DEADLINE);
    let doed_efter_ack = status(3, Some(id("ui", 5)), true); // nåede frem, døde bagefter
    let live = vec!["a".to_string(), "b".to_string()]; // c's mutex er væk igen

    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            Some(&doed_efter_ack),
            NU,
            false,
            &live,
            INGEN_ER_SYNLIG
        )),
        Action::TakeOver
    );
    assert_eq!(
        decide(&inputs(
            "b",
            &me,
            Some(&r),
            Some(&doed_efter_ack),
            NU,
            false,
            &live,
            INGEN_ER_SYNLIG
        )),
        Action::Nothing,
        "kun den laveste levende slug må tage over"
    );
}

#[test]
fn overtagelse_ogsaa_naar_deadline_er_loebet_ud_og_ingen_er_synlig() {
    // Den anden vej ind i samme gren: target nåede ALDRIG frem, og kilden er
    // forsvundet undervejs (crash lige efter den skjulte sig). Deadline er
    // overskredet, ingen levende er synlig — så skal laveste levende tage over
    // frem for at lade skærmen stå sort. Ingen anden test dækker den kombination
    // af "target har aldrig acket" og "TakeOver".
    let me = status(1, Some(id("ui", 4)), false);
    let r = req(id("ui", 5), "c", NU);
    let live = vec!["a".to_string(), "b".to_string()];
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            None,
            EFTER_DEADLINE,
            false,
            &live,
            INGEN_ER_SYNLIG
        )),
        Action::TakeOver
    );
}

#[test]
fn kold_start_af_target_maa_ikke_udloese_overtagelse_mens_en_kilde_er_synlig() {
    // REGRESSION (review fund 1, CRITICAL). Scenariet: `a` kører skjult, `b` er
    // synlig, `c` er stoppet. Brugeren klikker `c`; b skriver requesten og spawner
    // c. Ved næste 200 ms-tick booter c stadig — den navngivne mutex oprettes sent i
    // opstarten, så `target_alive` er falsk OG "c" mangler i `live_slugs`.
    //
    // Uden `!any_visible` læste a's gren (2) det som et dødt target, så sig selv som
    // laveste levende og returnerede TakeOver: a viste sig, b skjulte sig, og det
    // workspace brugeren faktisk bad om blev usynligt for evigt. Deterministisk, hver
    // gang min(live_slugs) er en skjult proces. Spec §3.3: "Fravær af filer må aldrig
    // udløse dead-target-værnet — kun launch_deadline gælder i opstartsvinduet."
    let me = status(1, Some(id("ui", 4)), false); // jeg er "a": kører, skjult
    let r = req(id("ui", 5), "c", EFTER_DEADLINE); // deadline IKKE overskredet
    let live = vec!["a".to_string(), "b".to_string()]; // c har ikke sin mutex endnu

    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            None,
            NU,
            false,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Nothing,
        "en bootende target må ikke ligne et dødt target mens kilden står fremme"
    );

    // Kontrol på præcis samme fixtur: det ENESTE der skiller de to er `any_visible`.
    // Forsvinder kilden (ingen levende er synlig), er overtagelsen igen den rigtige
    // handling — så asserten ovenfor måler synligheden og ikke bare "gør ingenting".
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            None,
            NU,
            false,
            &live,
            INGEN_ER_SYNLIG
        )),
        Action::TakeOver,
        "forsvinder den synlige kilde, skal laveste levende stadig kunne tage over"
    );
}

#[test]
fn tom_eller_ufuldstaendig_live_liste_maa_ikke_give_sort_skaerm() {
    // FUND 3: `live_slugs.iter().min() == Some(me)` fejler LUKKET når `me` ikke står i
    // listen. Ved en frisk installation, eller efter én fejlet dir-læsning i tick'et,
    // fyrer INGEN gren; vinduet er konfigureret `visible: false`, og skærmen er sort
    // indtil brugeren dræber processen. En ren funktion må ikke kunne producere en
    // uoprettelig sort skærm ud fra en input-antagelse den ikke selv kan håndhæve.
    let me = status(1, None, false);

    let tom: Vec<String> = Vec::new();
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            None,
            None,
            NU,
            false,
            &tom,
            INGEN_ER_SYNLIG
        )),
        Action::TakeOver,
        "en tom live-liste må ikke efterlade bootstrap-grenen handlingslammet"
    );

    // Samme fail-open i gren (2): listen kom ufuldstændig tilbage uden mig selv.
    let r = req(id("ui", 5), "c", EFTER_DEADLINE);
    let uden_mig = vec!["z".to_string()];
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            Some(&r),
            None,
            NU,
            false,
            &uden_mig,
            INGEN_ER_SYNLIG
        )),
        Action::TakeOver,
        "mangler jeg selv i listen, skal jeg stadig kunne redde fladen"
    );

    // Fail-open ophæver ikke determinismen: en HØJERE slug i listen betyder stadig
    // at nogen andre er lavere og skal tage over.
    assert_eq!(
        decide(&inputs(
            "z",
            &me,
            Some(&r),
            None,
            NU,
            false,
            &uden_mig,
            INGEN_ER_SYNLIG
        )),
        Action::TakeOver
    );
    let lavere = vec!["a".to_string()];
    assert_eq!(
        decide(&inputs(
            "z",
            &me,
            Some(&r),
            None,
            NU,
            false,
            &lavere,
            INGEN_ER_SYNLIG
        )),
        Action::Nothing,
        "er en lavere slug levende, må jeg ikke tage over"
    );
}

#[test]
fn bootstrap_naar_der_slet_ingen_request_findes() {
    // REGRESSION (plan-review P0-5): vinduet starter skjult (visible:false i
    // tauri.conf), så uden en request ved allerførste opstart ville skærmen
    // forblive sort for evigt.
    let me = status(1, None, false);
    let live = vec!["a".to_string()];
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            None,
            None,
            NU,
            false,
            &live,
            INGEN_ER_SYNLIG
        )),
        Action::TakeOver
    );

    // Men bootstrap-grenen må heller ikke rive fladen fra en synlig proces, hvis
    // active_workspace.json bare er blevet slettet under en kørende session.
    assert_eq!(
        decide(&inputs(
            "a",
            &me,
            None,
            None,
            NU,
            false,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Nothing,
        "en slettet request-fil må ikke lade en skjult slug overtage fra en synlig"
    );
}

#[test]
fn allerede_synlig_target_goer_intet() {
    let me = status(1, Some(id("ui", 5)), true);
    let r = req(id("ui", 5), "b", EFTER_DEADLINE);
    let live = vec!["b".to_string()];
    assert_eq!(
        decide(&inputs(
            "b",
            &me,
            Some(&r),
            None,
            NU,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Nothing
    );

    // POSITIV KONTROL i samme test: uden den ville `fn decide(_) -> Nothing` bestå.
    // Kvitteringen er for en ÆLDRE request, så jeg er ikke fremme for DENNE — samme
    // gren skal da afsløre sig.
    let ikke_fremme = status(1, Some(id("ui", 4)), true);
    assert_eq!(
        decide(&inputs(
            "b",
            &ikke_fremme,
            Some(&r),
            None,
            NU,
            true,
            &live,
            NOGEN_ER_SYNLIG
        )),
        Action::Reveal { request: r.clone() },
        "en kvittering for en ældre request tæller ikke som 'allerede fremme'"
    );
}

#[test]
fn tabt_active_fil_nulstiller_ikke_seq() {
    // MINOR 4: `read_active` mapper BÅDE fravær og korruption til `None`, så en
    // enkelt tabt eller halvskrevet fil sendte `seq` tilbage til 1. Samme proces
    // kunne dermed genudstede et `(issuer, seq)` den allerede havde brugt — stik
    // imod modul-doc'ens "parret er entydigt", og præcis det ABA-hul rollbackens
    // parvise sammenligning findes for at lukke.
    //
    // Vandmærket er proces-lokalt (statisk), så assertions er skrevet RELATIVT: de
    // er robuste mod at en fremtidig test i samme binary også kalder next_request,
    // men stadig røde mod den gamle `unwrap_or(0) + 1`.
    let dir = tempfile::tempdir().expect("temp global_base");
    let base = dir.path();
    write_active(base, &req(id("ui", 5), "b", EFTER_DEADLINE)).expect("skriv active");

    let foerste = next_request(base, "b", "inst-1", EFTER_DEADLINE.to_string());
    assert!(
        foerste.id.seq >= 6,
        "seq skal ligge over diskens 5, fik {}",
        foerste.id.seq
    );
    assert_eq!(foerste.id.issuer, "inst-1");
    assert_eq!(foerste.slug, "b");

    std::fs::remove_file(active_path(base)).expect("fjern active");
    let anden = next_request(base, "b", "inst-1", EFTER_DEADLINE.to_string());
    assert!(
        anden.id.seq > foerste.id.seq,
        "en tabt active_workspace.json må ikke lade samme proces genudstede et brugt seq \
         (fik {} efter {})",
        anden.id.seq,
        foerste.id.seq
    );
}

#[test]
fn persistensformaterne_taaler_manglende_felter() {
    // MINOR 7 / Global Constraint: "Persistens-formater er bagudkompatible".
    // Alle tre DTO'er skrives til disk. Uden `#[serde(default)]` ville en fil fra en
    // ældre build fejle i parseren, og fordi `read_active` sluger fejlen med `.ok()?`
    // ville resultatet se ud som "der findes ingen request" — altså en tavs
    // bootstrap/overtagelse i stedet for en synlig fejl.
    let r: RequestId = serde_json::from_str("{}").expect("RequestId uden felter");
    assert_eq!(r, id("", 0));

    let a: ActiveRequest =
        serde_json::from_str(r#"{"slug":"b"}"#).expect("ActiveRequest uden id/deadline");
    assert_eq!(a.slug, "b");
    assert_eq!(a.id, id("", 0));
    assert_eq!(a.launch_deadline, "");

    let s: WorkspaceStatus =
        serde_json::from_str(r#"{"pid":7}"#).expect("WorkspaceStatus uden nye felter");
    assert_eq!(s.pid, 7);
    assert!(
        !s.visible,
        "en fil uden visible-felt må aldrig læses som synlig"
    );
    assert_eq!(s.attention_kind, AttentionKind::None);
}

#[test]
fn legacy_attention_true_eskalerer_til_needs_you() {
    let s: WorkspaceStatus = serde_json::from_str(r#"{"attention":true}"#).unwrap();
    assert_eq!(
        s.attention_kind,
        AttentionKind::None,
        "feltet mangler paa den gamle wire"
    );
    assert_eq!(s.effective_attention_kind(), AttentionKind::NeedsYou);
}

#[test]
fn attention_kind_bruger_snake_case_paa_wiren() {
    let status = WorkspaceStatus {
        attention: true,
        attention_kind: AttentionKind::DoneUnread,
        ..Default::default()
    };
    let json = serde_json::to_string(&status).unwrap();
    assert!(json.contains(r#""attention_kind":"done_unread""#), "{json}");
    let roundtrip: WorkspaceStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(roundtrip.attention_kind, AttentionKind::DoneUnread);
}
