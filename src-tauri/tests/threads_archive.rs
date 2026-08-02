mod common;

use std::io::Write;
use talminal_canvas_lib::threads::archive;

fn write_thread(dir: &std::path::Path, name: &str, lines: &[&str]) {
    std::fs::create_dir_all(dir).unwrap();
    let mut file = std::fs::File::create(dir.join(name)).unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

/// Fixturerne haevdede foer ALLE sammen `"thread":"t9"`, ogsaa i filer der hed
/// t6/t7/t8. Det var praecis den forveksling passet led af, saa de kunne ikke
/// se den. En post hoerer til den fil den ligger i — helpers her tager derfor
/// traad-id'et som argument.
fn delegation(thread: &str) -> String {
    format!(
        r#"{{"seq":1,"thread":"{thread}","from_card":"card-1","from_kind":"agent","intent":"delegation","hop":1,"text":"lav X","ts_ms":1}}"#
    )
}

#[test]
fn startup_terminalizes_awaiting_threads_and_reports_count() {
    let _g = common::serial();
    let home = common::temp_home();
    let dir = home.path().join("threads");
    write_thread(&dir, "t9.jsonl", &[delegation("t9").as_str()]);

    assert_eq!(archive::terminalize_awaiting_on_startup().unwrap(), 1);

    let body = std::fs::read_to_string(dir.join("t9.jsonl")).unwrap();
    let last: serde_json::Value = serde_json::from_str(body.lines().last().unwrap()).unwrap();
    assert_eq!(last["from_kind"], "system");
    assert_eq!(last["terminal_reason"], "restart_abort");
    assert_eq!(last["seq"], 2);
    assert_eq!(last["thread"], "t9");
}

#[test]
fn startup_leaves_an_already_terminal_thread_alone() {
    let _g = common::serial();
    let home = common::temp_home();
    let dir = home.path().join("threads");
    write_thread(
        &dir,
        "t8.jsonl",
        &[
            delegation("t8").as_str(),
            r#"{"seq":2,"thread":"t8","from_card":"system","from_kind":"system","intent":"status","hop":1,"text":"vaek","ts_ms":2,"terminal_reason":"participant_lost"}"#,
        ],
    );
    assert_eq!(archive::terminalize_awaiting_on_startup().unwrap(), 0);
}

#[test]
fn an_answered_delegation_is_not_awaiting() {
    let _g = common::serial();
    let home = common::temp_home();
    let dir = home.path().join("threads");
    write_thread(
        &dir,
        "t7.jsonl",
        &[
            delegation("t7").as_str(),
            r#"{"seq":2,"thread":"t7","from_card":"card-2","from_kind":"agent","intent":"answer","hop":2,"text":"faerdig","ts_ms":2}"#,
        ],
    );
    assert_eq!(archive::terminalize_awaiting_on_startup().unwrap(), 0);
}

#[test]
fn a_corrupt_line_does_not_abort_the_pass() {
    let _g = common::serial();
    let home = common::temp_home();
    let dir = home.path().join("threads");
    write_thread(
        &dir,
        "t6.jsonl",
        &["{ dette er ikke json", delegation("t6").as_str()],
    );
    assert_eq!(archive::terminalize_awaiting_on_startup().unwrap(), 1);
}

#[test]
fn no_threads_dir_is_zero_not_an_error() {
    let _g = common::serial();
    let _home = common::temp_home();
    assert_eq!(archive::terminalize_awaiting_on_startup().unwrap(), 0);
}

/// Passet laeste foer traad-id'et ud af postens `thread`-felt og brugte det
/// baade som afsender-id og som APPEND-MAAL. En `t6.jsonl` hvis poster paastod
/// `"thread":"t9"` fik derfor sin restart_abort-linje skrevet i `t9.jsonl` og
/// terminaliserede dermed en FREMMED samtale — som her endda staar aaben og
/// ubesvaret bagefter. Filnavnet er sandheden; posten er data fra disken.
#[test]
fn a_post_that_claims_another_thread_cannot_terminalize_that_file() {
    let _g = common::serial();
    let home = common::temp_home();
    let dir = home.path().join("threads");
    write_thread(&dir, "t6.jsonl", &[delegation("t9").as_str()]);

    assert_eq!(
        archive::terminalize_awaiting_on_startup().unwrap(),
        0,
        "en post der ikke hoerer til filen maa ikke bestemme dens tilstand"
    );
    assert!(
        !dir.join("t9.jsonl").exists(),
        "indholdet i t6 skrev i en anden traads arkiv"
    );
    let body = std::fs::read_to_string(dir.join("t6.jsonl")).unwrap();
    assert_eq!(
        body.lines().filter(|l| !l.trim().is_empty()).count(),
        1,
        "t6 fik heller ikke selv en linje af den fremmede post"
    );
}

/// En fil hvis navn ikke er et traad-id springes over. Uden navne-vagten kunne
/// et vilkaarligt filnavn i mappen goere sig til et traad-id.
#[test]
fn a_file_whose_name_is_not_a_thread_id_is_skipped() {
    let _g = common::serial();
    let home = common::temp_home();
    let dir = home.path().join("threads");
    write_thread(&dir, "noter.jsonl", &[delegation("noter").as_str()]);

    assert_eq!(archive::terminalize_awaiting_on_startup().unwrap(), 0);
    let body = std::fs::read_to_string(dir.join("noter.jsonl")).unwrap();
    assert_eq!(body.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

/// `thread_path` joiner sit argument paa `threads/`. Uden valideringen dér
/// ville en streng fra en fremmed kilde kunne skrive uden for mappen, uanset
/// hvilken kaldevej der bragte den hertil — og en post uden `thread`-felt gav
/// en fil der bare hed `.jsonl`.
#[test]
fn thread_path_only_answers_for_real_thread_ids() {
    // Stien oploeses fra env paa kaldetidspunktet — laasen holder den stille.
    let _g = common::serial();
    assert!(archive::thread_path("t1").is_some());
    assert!(archive::thread_path("t42").is_some());
    for bad in [
        "", "..", "../../x", "t", "t1/../x", "tx", "t+1", ".", "t1.jsonl",
    ] {
        assert!(
            archive::thread_path(bad).is_none(),
            "{bad:?} er ikke et traad-id"
        );
    }
    let path = archive::thread_path("t7").unwrap();
    assert_eq!(path.parent().unwrap(), archive::threads_dir());
}
