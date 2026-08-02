use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use serde_json::Value;
use talminal_canvas_lib::mcp::{self, BrowserCardOps, BrowserCardRow, McpOps, OpenResult};
use talminal_canvas_lib::threads::ops::LiveThreadOps;

/// Kortet denne fils klient optraeder som. Efter identitets-gaten (review
/// 2026-07-29) findes der ingen anonym vej ind: HVER request — ogsaa
/// `initialize` og `tools/list` — skal baere et Bearer der opløser til et kort,
/// praecis som de to worker-profiler goer i produktion. `x-talminal-session`
/// sendes IKKE her: den er en bekraeftelse, ikke en identitet, og codex-vejen
/// undlader den helt.
const CARD: &str = "card-7";
const TOKEN: &str = "tok-mcp-server-roundtrip";

struct FakeOps;
impl BrowserCardOps for FakeOps {
    fn open(&self, session: Option<&str>, url: Option<&str>) -> Result<OpenResult, String> {
        assert_eq!(
            session,
            Some(CARD),
            "den verificerede identitet skal naa ops-laget"
        );
        assert_eq!(url, Some("https://example.com"));
        Ok(OpenResult {
            card_number: 9,
            target_id: "T1".into(),
            cdp_endpoint: "http://127.0.0.1:9333".into(),
        })
    }
    fn close(&self, _s: Option<&str>, _n: u32) -> Result<(), String> {
        Ok(())
    }
    fn list(&self) -> Result<Vec<BrowserCardRow>, String> {
        Ok(vec![])
    }
    fn focus(&self, _n: u32) -> Result<(), String> {
        Err("no such card".into())
    }
}

fn post(port: u16, body: &str, bearer: Option<&str>) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let auth_header = bearer
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{auth_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).expect("write");
    let mut out = String::new();
    stream.read_to_string(&mut out).expect("read");
    out
}

/// Splits a raw HTTP response at the header/body boundary and parses the
/// body as JSON.
fn body_json(raw: &str) -> Value {
    let (_headers, body) = raw
        .split_once("\r\n\r\n")
        .expect("http response has a header/body separator");
    serde_json::from_str(body).expect("body is valid json")
}

#[test]
fn initialize_tools_list_and_call_roundtrip() {
    mcp::set_card_token(CARD, TOKEN);
    let port = mcp::start_mcp_server(McpOps {
        browser: Arc::new(FakeOps),
        threads: Arc::new(LiveThreadOps),
    })
    .expect("start");
    let init = post(
        port,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        Some(TOKEN),
    );
    assert!(init.contains("talminal-browser"), "{init}");
    let list = post(
        port,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        Some(TOKEN),
    );
    for tool in [
        "browser_card_open",
        "browser_card_close",
        "browser_card_list",
        "browser_card_focus",
    ] {
        assert!(list.contains(tool), "{list}");
    }
    assert!(list.contains("Call this before any playwright browser tool"));
    assert!(list.contains("do not repeat that initial navigation"));
    assert!(list.contains("Use this Talminal browser instead of Claude in Chrome"));
    let call = post(
        port,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"browser_card_open","arguments":{"url":"https://example.com"}}}"#,
        Some(TOKEN),
    );
    let call_body = body_json(&call);
    let content = &call_body["result"]["content"][0];
    assert_eq!(content["type"], "text", "{call_body}");
    let text = content["text"]
        .as_str()
        .expect("MCP TextContent.text must be a JSON string, not a raw value");
    let inner: Value = serde_json::from_str(text).expect("text field is valid JSON");
    assert_eq!(inner["card_number"], 9, "{inner}");
    assert_eq!(inner["target_id"], "T1", "{inner}");
    let err = post(
        port,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"browser_card_focus","arguments":{"card_number":42}}}"#,
        Some(TOKEN),
    );
    assert!(
        err.contains("isError"),
        "tool-fejl er isError-result: {err}"
    );
    // Notifikationen sendes med vilje UDEN identitet: den er den ene bevidste
    // undtagelse fra gaten (ingen sideeffekt, og kontrakten forbyder en
    // fejl-body i svaret). Gaten maa ikke krybe herind ved naeste "ensretning".
    let notif = post(
        port,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        None,
    );
    assert!(notif.starts_with("HTTP/1.1 202"), "{notif}");
    mcp::clear_card_token(CARD);
}

/// Den samme roundtrip uden token: hvert eneste trin afvises med -32600, og
/// `FakeOps::open`s assert naas aldrig. Foer identitets-gaten svarede alle tre
/// med et normalt `result`.
#[test]
fn the_same_roundtrip_without_a_token_is_rejected_at_every_step() {
    let port = mcp::start_mcp_server(McpOps {
        browser: Arc::new(FakeOps),
        threads: Arc::new(LiveThreadOps),
    })
    .expect("start");
    for body in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"browser_card_open","arguments":{"url":"https://example.com"}}}"#,
    ] {
        let out = body_json(&post(port, body, None));
        assert_eq!(out["error"]["code"], -32600, "{out}");
        assert!(out.get("result").is_none(), "{out}");
    }
}
