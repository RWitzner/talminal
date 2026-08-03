//! T14: de tre traad-tools over den AEGTE HTTP-flade. En intern seam ville
//! ikke bevise noget om header-vejen — og header-vejen *er* paastanden.

mod common;

use serde_json::Value;
use std::sync::{Arc, Mutex};
use talminal_canvas_lib::mcp::{self, BrowserCardOps, BrowserCardRow, McpOps, OpenResult};
use talminal_canvas_lib::threads::pair::{self, SpawnedCard, Spawner};
use talminal_canvas_lib::threads::policy::AcceptsFromView;
use talminal_canvas_lib::threads::{self, FromKind, Intent, PostRequest};

struct NoBrowser;
impl BrowserCardOps for NoBrowser {
    fn open(&self, _s: Option<&str>, _u: Option<&str>) -> Result<OpenResult, String> {
        Err("not used".into())
    }
    fn close(&self, _s: Option<&str>, _n: u32) -> Result<(), String> {
        Ok(())
    }
    fn list(&self) -> Result<Vec<BrowserCardRow>, String> {
        Ok(vec![])
    }
    fn focus(&self, _n: u32) -> Result<(), String> {
        Err("not used".into())
    }
}

/// Parset svar. Selve request-bygningen bor i `common::mcp_post_raw` — den
/// form ER serverens accepterede flade og maa kun findes ét sted.
fn post(port: u16, body: &str, session: Option<&str>, bearer: Option<&str>) -> Value {
    common::body_json(&common::mcp_post_raw(port, body, session, bearer))
}

fn say_body() -> String {
    r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"card_say","arguments":{"thread":"t1","text":"hej","intent":"sparring"}}}"#.into()
}

fn is_error(v: &Value) -> bool {
    v["result"]["isError"] == Value::Bool(true)
}

fn error_text(v: &Value) -> String {
    v["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn started() -> u16 {
    mcp::start_mcp_server(McpOps {
        browser: Arc::new(NoBrowser),
        threads: Arc::new(talminal_canvas_lib::threads::ops::LiveThreadOps),
    })
    .expect("start")
}

fn open_thread_between(a: &str, b: &str) {
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    threads::create_thread("t1", "p", vec![a.into(), b.into()]).unwrap();
}

#[test]
fn bearer_only_ignores_the_session_header_entirely() {
    let _g = common::serial();
    assert_eq!(mcp::card_from_bearer_only(None), None);
    assert_eq!(mcp::card_from_bearer_only(Some("Basic abc")), None);
    assert_eq!(
        mcp::card_from_bearer_only(Some("Bearer ukendt-token")),
        None
    );
    mcp::set_card_token("card-9", "tok-known");
    assert_eq!(
        mcp::card_from_bearer_only(Some("Bearer tok-known")).as_deref(),
        Some("card-9")
    );
    mcp::clear_card_token("card-9");
}

#[test]
fn the_session_header_alone_cannot_speak_in_a_thread() {
    let _g = common::serial();
    open_thread_between("card-1", "card-2");
    let port = started();
    // Praecis det hul rev 1.1 lukkede: en uverificeret header maa ikke kunne
    // udpege afsenderen af en agent-til-agent-besked. H2 flyttede afvisningen
    // HELT ud af tool-laget — en header uden token-daekning er nu en afvist
    // REQUEST, saa `card_say` bliver aldrig dispatchet.
    let out = post(port, &say_body(), Some("card-1"), None);
    assert_eq!(out["error"]["code"], -32600, "{out}");
    assert!(out.get("result").is_none(), "{out}");
    assert_eq!(
        threads::get("t1").unwrap().messages.len(),
        0,
        "intet blev postet"
    );
}

#[test]
fn an_anonymous_caller_cannot_speak_in_a_thread() {
    let _g = common::serial();
    open_thread_between("card-1", "card-2");
    let port = started();
    let out = post(port, &say_body(), None, None);
    // Foer identitets-gaten var dette en TOOL-fejl: requesten var accepteret og
    // dispatchet, og kun `card_say`s egen arm sagde fra. Nu naar den slet ikke
    // ind — samme konklusion, et lag tidligere.
    assert_eq!(out["error"]["code"], -32600, "{out}");
    assert!(
        out.get("result").is_none(),
        "afvist foer tool-dispatchen: {out}"
    );
    assert_eq!(
        threads::get("t1").unwrap().messages.len(),
        0,
        "intet blev postet"
    );
}

#[test]
fn a_valid_bearer_token_may_speak_and_gets_hop_accounting_back() {
    let _g = common::serial();
    open_thread_between("card-1", "card-2");
    mcp::set_card_token("card-1", "tok-1");
    let port = started();
    let out = post(port, &say_body(), None, Some("tok-1"));
    assert!(!is_error(&out), "{out}");
    let payload: Value =
        serde_json::from_str(out["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["accepted"], true);
    assert_eq!(payload["hop"], 1);
    assert_eq!(payload["hops_left"], threads::MAX_HOPS - 1);
    assert_eq!(threads::get("t1").unwrap().messages.len(), 1);
    mcp::clear_card_token("card-1");
}

#[test]
fn a_tool_level_failure_is_an_iserror_content_not_a_jsonrpc_error() {
    let _g = common::serial();
    open_thread_between("card-1", "card-2");
    mcp::set_card_token("card-9", "tok-9"); // ikke medlem
    let port = started();
    let out = post(port, &say_body(), None, Some("tok-9"));
    assert!(
        out.get("error").is_none(),
        "husets form: aldrig JSON-RPC error for en tool-fejl"
    );
    assert!(is_error(&out));
    assert!(
        error_text(&out).contains("not a member"),
        "{}",
        error_text(&out)
    );
    mcp::clear_card_token("card-9");
}

#[test]
fn card_inbox_returns_the_batch_and_acks_through() {
    let _g = common::serial();
    open_thread_between("card-1", "card-2");
    threads::post(PostRequest {
        thread: "t1".into(),
        from_card: "card-1".into(),
        from_kind: FromKind::Agent,
        intent: Intent::Sparring,
        text: "linje\nmed\nlinjeskift og ```kode```".into(),
    })
    .unwrap();
    mcp::set_card_token("card-2", "tok-2");
    let port = started();

    let out = post(
        port,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"card_inbox","arguments":{"thread":"t1"}}}"#,
        None,
        Some("tok-2"),
    );
    let payload: Value =
        serde_json::from_str(out["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["messages"].as_array().unwrap().len(), 1);
    assert_eq!(payload["has_more"], false);
    // Flerlinjet indhold og kodeblokke krydser uden escaping (succeskriterium 7).
    let text = payload["messages"][0]["text"].as_str().unwrap();
    assert!(text.contains('\n') && text.contains("```"));

    let batch_id = payload["batch_id"].as_u64().unwrap();
    let acked = post(
        port,
        &format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"card_inbox","arguments":{{"thread":"t1","ack_through":{batch_id}}}}}}}"#
        ),
        None,
        Some("tok-2"),
    );
    let payload: Value =
        serde_json::from_str(acked["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(payload["messages"].as_array().unwrap().is_empty());
    mcp::clear_card_token("card-2");
}

#[test]
fn an_unknown_thread_is_an_error_not_a_panic() {
    let _g = common::serial();
    open_thread_between("card-1", "card-2");
    mcp::set_card_token("card-1", "tok-1");
    let port = started();
    let out = post(
        port,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"card_say","arguments":{"thread":"nope","text":"x","intent":"sparring"}}}"#,
        None,
        Some("tok-1"),
    );
    assert!(is_error(&out));
    assert!(error_text(&out).contains("nope"));
    mcp::clear_card_token("card-1");
}

#[test]
fn an_unknown_intent_is_rejected_with_the_valid_set() {
    let _g = common::serial();
    open_thread_between("card-1", "card-2");
    mcp::set_card_token("card-1", "tok-1");
    let port = started();
    let out = post(
        port,
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"card_say","arguments":{"thread":"t1","text":"x","intent":"kommando"}}}"#,
        None,
        Some("tok-1"),
    );
    assert!(is_error(&out));
    let text = error_text(&out);
    assert!(
        text.contains("sparring") && text.contains("answer"),
        "fejlen skal opregne det gyldige saet: {text}"
    );
    mcp::clear_card_token("card-1");
}

#[test]
fn tools_list_advertises_the_three_tools_and_names_the_reply_path() {
    let _g = common::serial();
    // tools/list kraever ogsaa identitet (identitets-gaten er ensartet).
    mcp::set_card_token("card-1", "tok-list");
    let port = started();
    let list = post(
        port,
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/list"}"#,
        None,
        Some("tok-list"),
    );
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in ["card_pair", "card_say", "card_inbox", "browser_card_open"] {
        assert!(names.contains(&expected), "{expected} mangler i tools/list");
    }
    // Lag-3-laeren: beskrivelsen skal sige hvad man goer BAGEFTER, ellers er
    // Buzz' #1-fejl (glemt callback) indbygget.
    let inbox = tools.iter().find(|t| t["name"] == "card_inbox").unwrap();
    let desc = inbox["description"].as_str().unwrap();
    assert!(
        desc.contains("card_say"),
        "card_inbox skal navngive svarvejen: {desc}"
    );
    assert!(desc.contains("ack_through"), "og kvitteringsvejen: {desc}");
    mcp::clear_card_token("card-1");
}

#[derive(Default)]
struct RecordingSpawner {
    agent: Mutex<String>,
}

impl Spawner for RecordingSpawner {
    fn spawn(&self, agent: &str, _cwd: &str) -> Result<SpawnedCard, String> {
        *self.agent.lock().unwrap() = agent.to_string();
        Ok(SpawnedCard {
            name: "partner-1".to_string(),
            agent: agent.to_string(),
        })
    }
    fn verify_running_with_mcp(&self, _card: &str) -> Result<(), String> {
        Ok(())
    }
    fn close(&self, _card: &str) {}
}

/// Review-tilfoejelse: `card_pair` var den eneste af de tre arme uden en
/// wire-test, og de tre argumenter er `&str` i traek — en ombytning ville
/// kompilere rent. Intet andet lukker hullet: T16's race-case kalder
/// `pair::card_pair` direkte, og ende-til-ende-beviset er et MANUELT
/// runbook-punkt. Her paastaas hele afbildningen paa én gang.
#[test]
fn card_pair_maps_the_wire_arguments_in_the_right_order() {
    let _g = common::serial();
    threads::reset_for_test();
    threads::policy::set_fixed_for_test(AcceptsFromView::Any);
    pair::reset_for_test();
    let spawner = Arc::new(RecordingSpawner::default());
    pair::set_spawner(spawner.clone());
    mcp::set_card_token("card-1", "tok-pair");
    let port = started();

    let out = post(
        port,
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"card_pair","arguments":{"agent":"codex","purpose":"spar om submit","opening_message":"Jeg foreslaar noget"}}}"#,
        None,
        Some("tok-pair"),
    );
    assert!(!is_error(&out), "{out}");
    let payload: Value =
        serde_json::from_str(out["result"]["content"][0]["text"].as_str().unwrap()).unwrap();

    // Afsenderen kommer fra det VERIFICEREDE Bearer-token, ikke fra et argument.
    let thread = payload["thread"].as_str().unwrap();
    let t = threads::get(thread).unwrap();
    assert!(
        t.members.contains(&"card-1".to_string()),
        "from_card skal komme fra tokenet: {:?}",
        t.members
    );
    // agent -> spawneren, purpose -> traadens etiket, opening_message -> foerste besked.
    assert_eq!(*spawner.agent.lock().unwrap(), "codex");
    assert_eq!(t.purpose, "spar om submit");
    assert_eq!(t.messages[0].text, "Jeg foreslaar noget");
    assert_eq!(payload["partner"], "partner-1");

    mcp::clear_card_token("card-1");
    talminal_canvas_lib::registry::close_card(payload["chat_card"].as_str().unwrap().to_string())
        .ok();
}

#[test]
fn there_is_no_tool_that_mutates_the_policy() {
    let _g = common::serial();
    mcp::set_card_token("card-1", "tok-policy");
    let port = started();
    let list = post(
        port,
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#,
        None,
        Some("tok-policy"),
    );
    let blob = list.to_string();
    // Invarianten fra spec §4.3: politikken er ikke agent-muterbar. Skrives den
    // ikke som en test, bliver gaten dekorativ ved naeste "billige tilfoejelse".
    assert!(
        !blob.contains("accepts_from"),
        "intet tool maa eksponere politikken"
    );
    // ... og listen skal faktisk vaere leveret, ikke en -32600-afvisning der
    // trivielt ikke indeholder ordet.
    assert!(
        list["result"]["tools"]
            .as_array()
            .is_some_and(|t| !t.is_empty()),
        "{list}"
    );
    mcp::clear_card_token("card-1");
}
