//! Minimal streamable-HTTP MCP server (browser-cards plan Task 4).
//!
//! Deterministic hand-rolled JSON-RPC 2.0 over `tiny_http` instead of the
//! `rmcp` SDK (plan-afgørelse, spec §6-amendment: stateless, 4
//! `browser_card_*` tools; handshake mod ægte Claude Code verificeret i
//! spike S0). Fully decoupled from the rest of the browser-cards stack: all
//! card effects flow through `BrowserCardOps`, injected by the caller (Task
//! 5 wires the real registry-backed implementation; Task 6 bakes the
//! per-worker `x-talminal-session` header into the generated worker
//! configs).
//!
//! Protocol (binding — plan Task 4 / spec §6): POST `/mcp` with JSON-RPC
//! 2.0. `initialize` echoes the caller's `protocolVersion` (or defaults to
//! [`PROTOCOL_VERSION`]). `notifications/*` -> HTTP 202, empty body.
//! `tools/list` returns the tool set. `tools/call` ->
//! `{content:[{type:"text",text:<result>}]}`, or on tool failure
//! `{content:[...],isError:true}` (still a successful JSON-RPC `result` —
//! tool errors are not protocol errors). This paragraph is the ONE status
//! contract; everything else — notably [`empty_json_response`] — points here
//! instead of keeping a second copy that can rot apart from it:
//!
//! * any path other than `/mcp` -> 404; `GET /mcp` -> 405
//! * a request carrying an `Origin` header -> 403, before the body is read
//! * a body over [`MAX_BODY_BYTES`] -> 413
//! * an identity we cannot verify — absent, unparsable or stale -> `-32600`
//!   ([`SessionLookup`]); `notifications/*` are the single exemption, see
//!   [`handle_request`]
//! * unknown JSON-RPC method -> `-32601`; body parse failure -> `-32700`
//!
//! The single server thread never panics on a bad request — it logs and keeps
//! serving.
//!
//! Identitet og vaern (review 2026-07-29, fund H2/M10 + kritiker-opfoelgning):
//! identiteten kommer fra det VERIFICEREDE `Authorization: Bearer`-token —
//! `x-talminal-session` er en uverificeret klient-paastand og gaelder kun som
//! bekraeftelse af tokenets kort ([`SessionLookup`]). HVER request skal baere en
//! identitet, ogsaa `initialize` og `tools/list`: begge worker-profiler saetter
//! den paa alle deres kald (`worker_mcp::write_config` skriver headeren i CC's
//! config, `codex_launch_args` peger codex paa token-env'en), og canvas selv
//! kalder `browser_host` IN-PROCESS i Rust — HTTP-fladen har derfor ingen
//! legitim anonym klient. Origin-vaernet staar tilbage som baelte og seler.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
#[cfg(feature = "perf-trace")]
use std::time::Instant;

use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

/// Actual bound port of the running MCP server (set once, in
/// [`start_mcp_server`]).
static MCP_PORT: OnceLock<u16> = OnceLock::new();

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "talminal-browser";
const SERVER_VERSION: &str = "1.0.0";
const SESSION_HEADER: &str = "x-talminal-session";
const AUTHORIZATION_HEADER: &str = "Authorization";
const ORIGIN_HEADER: &str = "Origin";
/// Loft for request-bodyen (fund M10). Den stoerste legitime payload er
/// `card_say`, og den har sit eget loft paa 16 KiB
/// (`threads::MAX_MESSAGE_BYTES`) — 1 MiB ligger derfor to stoerrelsesordener
/// fra enhver aegte klient og rammer kun den der lyver om sin Content-Length.
const MAX_BODY_BYTES: usize = 1024 * 1024;
#[cfg(feature = "perf-trace")]
const PERF_HEADER: &str = "x-talminal-perf";

/// Kort-nøglet Bearer-token-registry (GPT-review B3): `card_name -> token`,
/// ikke `token -> card_name` — close/kill kender kortnavnet, ikke tokenet.
/// `set_card_token` erstatter et evt. eksisterende token for kortet (et
/// respawn invaliderer dermed automatisk det gamle); `clear_card_token`
/// fjerner det (main.rs kalder den paa ALLE afslutningsveje). Bearer-opslaget
/// er en lineaer soegning over vaerdierne — N = antal kort, altid lille.
static CARD_TOKENS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn card_tokens() -> &'static Mutex<HashMap<String, String>> {
    CARD_TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Saetter/erstatter kortets token. Groebet holdes kort og IKKE nestet med
/// andre kort-token-laase (enkelt-traadet request-loop, se modul-docs).
pub fn set_card_token(card_name: &str, token: &str) {
    let mut tokens = card_tokens().lock().expect("card-token mutex poisoned");
    tokens.insert(card_name.to_string(), token.to_string());
}

/// Fjerner kortets token — kaldes fra main.rs paa alle afslutningsveje
/// (fejlet spawn, kill_card, close_card/close_cards, proces-exit).
pub fn clear_card_token(card_name: &str) {
    let mut tokens = card_tokens().lock().expect("card-token mutex poisoned");
    tokens.remove(card_name);
}

/// Har kortet et gyldigt Bearer-token lige nu? `card_pair` bruger den som
/// bevis for at MCP-injektionen skete — et kort uden token kan ikke kalde de
/// tre traad-tools og er derfor ubrugeligt som partner (beslutning 12).
pub fn has_card_token(card_name: &str) -> bool {
    card_tokens()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains_key(card_name)
}

fn card_for_token(token: &str) -> Option<String> {
    let tokens = card_tokens().lock().expect("card-token mutex poisoned");
    tokens
        .iter()
        .find(|(_, v)| v.as_str() == token)
        .map(|(k, _)| k.clone())
}

/// Bearer-only identitet til traad-tools. Ser IKKE paa x-talminal-session:
/// den header er ikke verificeret og maa derfor ikke kunne udpege afsenderen af
/// en agent-til-agent-besked.
///
/// Naar en request slipper igennem identitets-gaten i [`handle_request`], giver
/// denne og [`session_from_headers`] nu ALTID samme kort — begge opløser samme
/// Bearer mod samme registry. De to udledninger bliver alligevel staaende hver
/// for sig, fordi de svarer paa hver sit spoergsmaal: den ene er browser-tools'
/// scope-noegle (`BrowserCardOps` tager `Option`, fordi main.rs' IN-PROCESS-kald
/// med rette ikke har nogen ejer), den anden er traad-tools' afsender-bevis.
/// Foldes de sammen, arver browser-tools tavst traad-tools' kontrakt naeste gang
/// en af dem aendrer sig.
pub fn card_from_bearer_only(authorization_header: Option<&str>) -> Option<String> {
    let token = authorization_header?.strip_prefix("Bearer ")?;
    card_for_token(token)
}

/// Firedelt sessionsudledning. Identiteten kommer fra det VERIFICEREDE
/// Bearer-token; `x-talminal-session` er en uverificeret klient-paastand og
/// taeller derfor kun som BEKRAEFTELSE af det kort tokenet opløser til (H2).
///
/// * `Card` — Bearer opløser til et kort, og session-headeren er enten fravaerende
///   (codex-vejen sender kun Bearer) eller identisk med kortnavnet (CC-vejen
///   sender begge: `worker_mcp::write_config` skriver headeren, og `main.rs`
///   registrerer tokenet under praecis samme `name`, saa de matcher altid).
/// * `UnverifiedSession` — en session-header uden daekning i et token: enten
///   helt uden `Authorization`-header, eller med et Bearer der opløser til et
///   ANDET kort. Foer H2 vandt headeren i begge de tilfaelde, saa enhver lokal
///   proces kunne aabne og lukke browser-kort i et vilkaarligt korts navn. Den
///   maa heller ikke stiltiende degradere til `Anonymous`: det ville *forfremme*
///   den afviste paastand til den privilegerede session. (Er `Authorization`
///   derimod til stede men uparsebar, er svaret `InvalidToken` — se nedenfor.)
/// * `InvalidToken` — et Bearer vi ikke kan opløse (GPT-review B4): ukendt,
///   udloebet, ELLER en `Authorization`-header der slet ikke er et Bearer (fx
///   `Basic xyz`). Degraderer ALDRIG til `Anonymous` — en identitet vi ikke kan
///   verificere maa ikke arve privilegiet. Netop `Basic xyz` faldt foer gennem
///   `strip_prefix` til `None` og landede — uden session-header — praecis dér.
/// * `Anonymous` — ingen identitets-headers overhovedet. Udledningen beholder
///   navnet, men [`handle_request`] AFVISER den nu med `-32600`: sessionen er
///   PRIVILEGERET (browser_host.rs lader den aabne canvas-ejede kort og lukke
///   `opened_by=None`-kort), og gennemsoegningen af repoet 2026-07-29 fandt
///   ingen legitim klient der rammer HTTP-fladen uden identitet — canvas kalder
///   `browser_host` in-process, og begge worker-profiler baerer Bearer. Vejen
///   var altsaa udelukkende den en fjendtlig webside ville forsoege.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLookup {
    Anonymous,
    Card(String),
    UnverifiedSession,
    InvalidToken,
}

fn session_from_headers(
    session_header: Option<&str>,
    authorization_header: Option<&str>,
) -> SessionLookup {
    let token = match authorization_header {
        // En Authorization-header vi ikke kan PARSE er en identitet vi ikke kan
        // verificere, ikke en fravaerende identitet. Sammenkaeden
        // `and_then(strip_prefix)` slog de to sammen, saa "Basic xyz" endte i
        // `Anonymous` — B4-reglen skal staa her i udledningen, ikke kun i
        // afvisningslaget.
        Some(header) => match header.strip_prefix("Bearer ") {
            Some(token) => token,
            None => return SessionLookup::InvalidToken,
        },
        // Uden token er der intet at bekraefte headeren MOD.
        None => {
            return match session_header {
                Some(_) => SessionLookup::UnverifiedSession,
                None => SessionLookup::Anonymous,
            }
        }
    };
    match (card_for_token(token), session_header) {
        (None, _) => SessionLookup::InvalidToken,
        (Some(card), None) => SessionLookup::Card(card),
        (Some(card), Some(claimed)) if claimed == card => SessionLookup::Card(card),
        // Mismatch er enten forfalskning eller konfigurations-drift; begge dele
        // skal hoeres, ikke tavst opløses til ét af de to kort.
        (Some(_), Some(_)) => SessionLookup::UnverifiedSession,
    }
}

/// Result of `browser_card_open` (binding for Task 5/6).
pub struct OpenResult {
    pub card_number: u32,
    pub target_id: String,
    pub cdp_endpoint: String,
}

/// One row of `browser_card_list` (binding for Task 5/6).
pub struct BrowserCardRow {
    pub number: u32,
    pub opened_by: Option<String>,
    pub url: String,
    pub title: String,
    pub target_id: String,
}

/// Card-effect boundary the MCP layer calls through. Kept decoupled from
/// the registry/webview-host modules on purpose — Task 5 supplies the real
/// implementation; this module imports nothing from it.
pub trait BrowserCardOps: Send + Sync + 'static {
    fn open(&self, session: Option<&str>, url: Option<&str>) -> Result<OpenResult, String>;
    fn close(&self, session: Option<&str>, card_number: u32) -> Result<(), String>;
    fn list(&self) -> Result<Vec<BrowserCardRow>, String>;
    fn focus(&self, card_number: u32) -> Result<(), String>;
}

/// Traad-effekternes graense (plan T14). Den aegte implementation er
/// `threads::ops::LiveThreadOps`; testene injicerer deres egen. Argumenterne
/// er raa wire-vaerdier — oversaettelsen til domaenetyper (fx `Intent`) er
/// implementationens beslutning, ikke MCP-lagets.
pub trait ThreadOps: Send + Sync + 'static {
    fn pair(
        &self,
        from_card: &str,
        agent: &str,
        purpose: &str,
        opening: &str,
    ) -> Result<Value, String>;
    fn say(&self, from_card: &str, thread: &str, text: &str, intent: &str)
        -> Result<Value, String>;
    fn inbox(&self, card: &str, thread: &str, ack_through: Option<u64>) -> Result<Value, String>;
}

/// De to effekt-grænser serveren kalder gennem. Samlet i én struct, saa
/// `start_mcp_server` ikke vokser en ny parameter for hver tool-familie.
pub struct McpOps {
    pub browser: Arc<dyn BrowserCardOps>,
    pub threads: Arc<dyn ThreadOps>,
}

/// Starts the MCP server bound to `127.0.0.1:0`, spawns its single
/// request-loop thread, and returns the actual bound port.
pub fn start_mcp_server(ops: McpOps) -> Result<u16, String> {
    let server = Server::http("127.0.0.1:0").map_err(|e| format!("mcp: bind failed: {e}"))?;
    let port = match server.server_addr().to_ip() {
        Some(addr) => addr.port(),
        None => return Err("mcp: bound address is not an IP socket".to_string()),
    };
    // Best-effort: if a server was already started earlier in this process,
    // MCP_PORT keeps its first value. Callers only rely on the `u16`
    // returned here to talk to *this* server instance.
    let _ = MCP_PORT.set(port);

    thread::spawn(move || {
        for request in server.incoming_requests() {
            #[cfg(feature = "perf-trace")]
            let request_id = request_header(&request, PERF_HEADER);
            #[cfg(feature = "perf-trace")]
            let request_started = Instant::now();
            crate::perf_mark_background!(
                "mcp.request.accepted",
                json!({ "port": port, "request_id": request_id.as_deref() }),
            );
            handle_request(request, &ops);
            crate::perf_mark_background!(
                "mcp.request.completed",
                json!({
                    "port": port,
                    "request_id": request_id.as_deref(),
                    "handler_ms": request_started.elapsed().as_secs_f64() * 1_000.0,
                }),
            );
        }
    });

    Ok(port)
}

/// The bound port of the last-started MCP server in this process, if any.
pub fn mcp_port() -> Option<u16> {
    MCP_PORT.get().copied()
}

fn handle_request(mut request: Request, ops: &McpOps) {
    if request.url() != "/mcp" {
        respond(request, empty_json_response(StatusCode(404)));
        return;
    }

    if *request.method() != Method::Post {
        respond(request, empty_json_response(StatusCode(405)));
        return;
    }

    // H2-vaernet mod localhost-CSRF: browser-kort viser pr. design vilkaarlige
    // websider, og en fjendtlig side kan sende no-preflight POSTs (text/plain)
    // mod loopback-porte uden at kunne laese svaret. Den kan derimod IKKE saette
    // custom headers — `Authorization`/`x-talminal-session` udloeser preflight,
    // som vi ikke besvarer — saa den kunne kun ramme den anonyme vej, og DEN er
    // nu lukket af identitets-gaten laengere nede. Vaernet bliver alligevel
    // staaende som baelte og seler: det er det eneste af de to lag der virker
    // FOER body-laesningen, og det ville stadig staa hvis en fremtidig
    // aendring gjorde en identitets-loes vej lovlig igen.
    // Browsere saetter til gengaeld altid `Origin` paa cross-origin POST,
    // og ingen MCP-klient saetter den (Node/undici springer headeren over naar
    // origin er "client", og Rust-klienter saetter den aldrig): en request med
    // Origin er pr. definition ikke en af vores. MCP-spec'en kraever selv
    // Origin-validering paa HTTP-transporten, saa vaernet er ikke husets
    // opfindelse. Afvisningen sker FOER body-laesningen, saa siden heller ikke
    // kan bruge os som parser — og den logges, for hvis en fremtidig klient
    // alligevel saetter Origin, ser fejlen udefra ud som total MCP-tavshed.
    if let Some(origin) = request_header(&request, ORIGIN_HEADER) {
        eprintln!("[mcp] rejected request carrying Origin: {origin}");
        respond(request, empty_json_response(StatusCode(403)));
        return;
    }

    // M10: en annonceret body over loftet afvises FOER vi laeser den, saa en
    // loegnagtig Content-Length ikke kan koebe vores parsing. FORBEHOLD, saa den
    // naeste laeser ikke tror hullet er lukket — begge dele verificeret i
    // tiny_http 0.12.0's kilde:
    //   (a) HUKOMMELSEN er IKKE bundet af vores 413. Svarer vi uden at laese,
    //       draener `EqualReader::drop` (util/equal_reader.rs:67-70) selv resten
    //       — og allokerer `vec![0; remaining_to_read]` for HELE den annoncerede
    //       rest paa én gang. Loftet bunder altsaa kun VORES egen String og
    //       JSON-parsing; crate'ens buffer foelger klientens tal.
    //   (b) TIDEN er slet ikke bundet. Crate'en eksponerer ingen socket-timeout,
    //       saa selv en LILLE annonceret body kan holde vores ENE request-traad
    //       aaben lige saa laenge klienten tier. Begge dele kraever en anden
    //       transport eller et flertraadet loop — en arkitektur-beslutning, ikke
    //       en rettelse.
    if let Some(announced) = request.body_length() {
        if announced > MAX_BODY_BYTES {
            eprintln!("[mcp] rejecting body of {announced} bytes (limit {MAX_BODY_BYTES})");
            respond(request, empty_json_response(StatusCode(413)));
            return;
        }
    }

    let session_header_value = session_header(&request);
    let authorization_header = request_header(&request, AUTHORIZATION_HEADER);
    // To udledninger fra samme request. Efter identitets-gaten nedenfor peger de
    // altid paa SAMME kort; hvorfor de alligevel staar hver for sig, staar i
    // [`card_from_bearer_only`]'s doc.
    let bearer_card = card_from_bearer_only(authorization_header.as_deref());

    let mut raw_body = String::new();
    // M10: `.take()` er den ENESTE graense paa de to veje hvor gaten ovenfor
    // ikke kan hjaelpe — og de er det BAERENDE ben her, ikke et ekstra lag oven
    // paa 413'eren (verificeret i tiny_http 0.12.0):
    //   * Transfer-Encoding: Content-Length ignoreres pr. RFC2616 §4.4
    //     (request.rs:150-153), saa `body_length()` er `None` og gaten springes
    //     helt over.
    //   * `Connection: upgrade`: body-readeren ER den raa socket
    //     (request.rs:188-190). Ingen laengde haandhaeves — heller ikke en
    //     Content-Length klienten selv annoncerede — saa gaten kan slippe en
    //     request igennem der derefter streamer ubegraenset.
    // Netop de to ben er utestede (413-testen nedenfor daekker den ANNONCEREDE
    // vej), saa den, der en dag "forenkler" `.take()` vaek, faar en groen suite.
    if let Err(e) = request
        .as_reader()
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_string(&mut raw_body)
    {
        eprintln!("[mcp] failed to read request body: {e}");
        respond(request, json_response(parse_error_response()));
        return;
    }
    if raw_body.len() > MAX_BODY_BYTES {
        eprintln!("[mcp] rejecting unannounced body over {MAX_BODY_BYTES} bytes");
        respond(request, empty_json_response(StatusCode(413)));
        return;
    }

    let value: Value = match serde_json::from_str(&raw_body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[mcp] parse error: {e}");
            respond(request, json_response(parse_error_response()));
            return;
        }
    };

    let method = value.get("method").and_then(Value::as_str).unwrap_or("");

    // notifications/* never get a JSON-RPC response body per the MCP
    // streamable-HTTP contract — just an empty 202. BEVIDST UNDTAGELSE fra
    // identitets-gaten nedenfor: en notifikation naar aldrig ops-laget og har
    // ingen sideeffekt, og kontrakten forbyder os at svare med en fejl-body —
    // der er altsaa hverken noget at beskytte eller noget at afvise MED.
    if method.starts_with("notifications/") {
        respond(request, empty_json_response(StatusCode(202)));
        return;
    }

    let id = value.get("id").cloned().unwrap_or(Value::Null);

    // IDENTITETS-GATEN. Alt andet end en verificeret Bearer-identitet er en
    // afvist request — den degraderer ALDRIG til den privilegerede None-session
    // (browser_host.rs' canvas-ejede/opened_by=None-undtagelser):
    //   * `InvalidToken` (B4): et Bearer vi ikke kan opløse.
    //   * `UnverifiedSession` (H2): en session-header uden token-daekning.
    //   * `Anonymous`: slet ingen identitet. Gaten daekker med vilje ogsaa
    //     `initialize` og `tools/list` — ensartet, fordi begge worker-profiler
    //     saetter identiteten paa ALLE deres kald (CC via config-headeren, codex
    //     via `bearer_token_env_var`) og tokenet er registreret foer processen
    //     spawnes (main.rs: `set_card_token` ligger foer `PtySpawn`). En
    //     halv-aaben handshake ville kun gavne den der ikke har et token.
    let session = match session_from_headers(
        session_header_value.as_deref(),
        authorization_header.as_deref(),
    ) {
        SessionLookup::InvalidToken => {
            // Alle tre arme logger. Uden en linje her ser en afvisning udefra ud
            // som total MCP-tavshed: workeren faar bare "MCP server failed" i
            // CC's `/mcp`, fordi gaten ogsaa daekker `initialize`. Kortnavn og
            // method er nok til at skelne "klienten fik aldrig injektionen" fra
            // "tokenet er udloebet" — tokenet selv logges ALDRIG.
            eprintln!("[mcp] rejected {method}: bearer not resolvable");
            respond(
                request,
                // Daekker nu ogsaa den uparsebare header (fx `Basic ...`), saa
                // beskeden navngiver alle tre maader et Bearer kan svigte paa.
                json_response(invalid_request_error(
                    id,
                    "Authorization must be a Bearer token for a live card: this one is \
                     unknown, expired, or not a Bearer at all",
                )),
            );
            return;
        }
        SessionLookup::UnverifiedSession => {
            let claimed = session_header_value.as_deref().unwrap_or("(ingen)");
            eprintln!(
                "[mcp] rejected {method}: session header {claimed} without a matching bearer"
            );
            respond(
                request,
                json_response(invalid_request_error(
                    id,
                    "x-talminal-session is not an identity: it must match the card its \
                     Authorization: Bearer token resolves to",
                )),
            );
            return;
        }
        SessionLookup::Anonymous => {
            eprintln!("[mcp] rejected {method}: no identity");
            respond(
                request,
                json_response(invalid_request_error(
                    id,
                    "identity required: every request must carry Authorization: Bearer \
                     <card token>",
                )),
            );
            return;
        }
        SessionLookup::Card(name) => name,
    };

    let response_body = match method {
        "initialize" => initialize_result(&value, id),
        "tools/list" => tools_list_result(id),
        "tools/call" => tools_call_result(
            &value,
            id,
            ops,
            Some(session.as_str()),
            bearer_card.as_deref(),
        ),
        _ => method_not_found_error(id),
    };

    respond(request, json_response(response_body));
}

fn respond<R: Read>(request: Request, response: Response<R>) {
    if let Err(e) = request.respond(response) {
        eprintln!("[mcp] failed to write response: {e}");
    }
}

fn session_header(request: &Request) -> Option<String> {
    request_header(request, SESSION_HEADER)
}

fn request_header(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

fn content_type_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static content-type header is always valid")
}

fn json_response(body: Value) -> Response<Cursor<Vec<u8>>> {
    Response::from_string(body.to_string())
        .with_status_code(StatusCode(200))
        .with_header(content_type_header())
}

/// Empty-body response for the statuses that carry no JSON-RPC envelope (202,
/// 403, 404, 405, 413) — the module doc's "Protocol (binding)" paragraph is the
/// contract; this doc deliberately does not restate which status means what.
/// It still carries the mandated `Content-Type: application/json` header:
/// `Response::empty` sets no headers on its own.
fn empty_json_response(status: StatusCode) -> Response<Cursor<Vec<u8>>> {
    Response::from_data(Vec::new())
        .with_status_code(status)
        .with_header(content_type_header())
}

fn parse_error_response() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": { "code": -32700, "message": "Parse error" }
    })
}

fn method_not_found_error(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32601, "message": "Method not found" }
    })
}

/// B4 (GPT-review) + H2: -32600-klassen ("Invalid Request") — en identitet vi
/// ikke kan verificere er en AFVIST request, aldrig en degraderet
/// Anonymous-session. Beskeden navngiver hvilken af de TRE maader identiteten
/// svigtede paa (ubrugeligt Bearer / uverificeret session-header / slet ingen
/// identitet), saa en fejlkonfigureret worker kan skelnes fra et udloebet token
/// og fra en klient der aldrig fik injektionen.
fn invalid_request_error(id: Value, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32600, "message": message }
    })
}

/// Staaende orientering til enhver worker der forbinder. MCP-standardens
/// `InitializeResult.instructions`, og den ENESTE kanal vi har til en spawnet
/// partner: `card_pair` starter den uden prompt (pair.rs), saa uden dette felt
/// er dens foerste input notitsen fra `notice_text` — én linje.
///
/// Begge klienter er verificeret empirisk (A/B med sentinel, 2026-07-29):
///   * Claude Code 2.1.220 injicerer teksten som en "# MCP Server Instructions"-
///     blok og AFKORTER HAARDT ved 2048 tegn pr. server. Derfor testen nedenfor.
///     Vigtigt: feltet lastes IVRIGT, mens tool-beskrivelser er deferred — saa
///     det her naar laengere end `tool_definitions()` gør.
///   * Codex CLI 0.145.0 laegger teksten som `description` paa namespace-toolet
///     `mcp__talminal`. Ingen haard afkortning, men OpenAIs egen anbefaling er
///     at de foerste 512 tegn skal kunne staa alene.
///
/// Begge peger samme vej: ét stramt afsnit, det dyreste foerst.
///
/// Indholdet er valgt efter hvad der KOSTER noget i dag. Wake-kontrakten staar
/// oeverst, fordi en agent der poller i stedet for at gaa til ro aldrig faar
/// notitsen (BUSY_QUIET_MS, main.rs) og faar traaden ryddet af backstoppen
/// (BACKSTOP_MS, dispatch.rs) uden nogensinde at have set delegeringen.
/// Browser-KONVENTIONERNE staar bevidst IKKE her — de bor i tool-beskrivelserne,
/// hvor de er relevante naar toolet alligevel slaas op. Den ENE browser-linje der
/// er med, handler om hvad man goer naar toolsene IKKE er der, og kan derfor pr.
/// definition ikke staa i en tool-beskrivelse: en agent der ikke fandt playwright
/// skrev sit eget cdp.mjs (browser-guidance-injection.md §1).
const SERVER_INSTRUCTIONS: &str = "\
Talminal. You run inside a card on a canvas a human is watching. This server lets you open browser cards and talk to a second agent through a thread.

WAKE CONTRACT - read this first. When a partner writes to a thread, Talminal types a notice into your terminal, but only once your card has been silent for 1.5 seconds. Keep working or poll in a loop and the notice never lands; an undelivered delegation to you is then swept away after 21 minutes. So when you wait on a partner: either poll card_inbox deliberately, or end your turn and go quiet. Do not watch files or spawn background waiters to work around this.

THREADS. card_say's intent is a state machine, not four labels. intent=delegation puts the thread in awaiting and blocks further delegations until the assignee answers with intent=answer; only that agent can clear it. Deadlines: 5 minutes idle, 20 minutes absolute - the absolute one fires even while you are busy. card_inbox does NOT mark messages read: pass the returned batch_id back as ack_through, or you get the same batch again. The hop budget is 20 and only agent messages count against it; the human's do not.

LIMITS. Two partner cards per app session, counted cumulatively - closing one does not give the slot back. A card that was itself spawned by a pairing may not pair again. 16 KiB per message, 5 messages per card_inbox call, 60 characters of purpose.

BROWSER. Call browser_card_open before any playwright tool - it creates the tab you then drive. If playwright tools are still unavailable after one search, say so and stop: never hand-roll CDP scripts against the endpoint.

Replying in your own terminal reaches nobody. Only card_say does.";

fn initialize_result(value: &Value, id: Value) -> Value {
    let protocol_version = value
        .get("params")
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str)
        .unwrap_or(PROTOCOL_VERSION);

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": protocol_version,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
            "instructions": SERVER_INSTRUCTIONS
        }
    })
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "browser_card_open",
            "description": "Opens a new browser card in the caller's Talminal scope and returns its card number, Playwright target id, and CDP endpoint. Call this before any playwright browser tool - it creates the tab you then drive with playwright. When url is provided, the card starts navigating to it immediately; do not repeat that initial navigation with a Playwright navigate call. Do not use playwright's browser_tabs to create tabs; open a new card instead. Use this Talminal browser instead of Claude in Chrome.",
            "inputSchema": {
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "additionalProperties": false
            }
        },
        {
            "name": "browser_card_close",
            "description": "Closes a browser card by number. Only the card's opener may close it.",
            "inputSchema": {
                "type": "object",
                "properties": { "card_number": { "type": "integer" } },
                "required": ["card_number"],
                "additionalProperties": false
            }
        },
        {
            "name": "browser_card_list",
            "description": "Lists every open browser card with its number, opener, url, title, and Playwright target id.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "browser_card_focus",
            "description": "Brings a browser card to the front of the canvas. Native focus only; select the matching playwright tab separately - focusing the card does not change which tab playwright is driving.",
            "inputSchema": {
                "type": "object",
                "properties": { "card_number": { "type": "integer" } },
                "required": ["card_number"],
                "additionalProperties": false
            }
        },
        {
            "name": "card_pair",
            "description": "Pairs you with a second agent: spawns a partner card, opens a thread between you two, creates the chat card the owner watches, and posts your opening_message. One call, no follow-up setup. Returns the thread id you then use with card_say and card_inbox. The partner replies through the thread - watch for it with card_inbox, never by reading its terminal.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "description": "Which agent to pair with, e.g. claude or codex." },
                    "purpose": { "type": "string", "description": "Short label for the thread, shown to the owner." },
                    "opening_message": { "type": "string", "description": "The first message, posted into the thread on your behalf." }
                },
                "required": ["agent", "purpose", "opening_message"],
                "additionalProperties": false
            }
        },
        {
            "name": "card_say",
            "description": "Posts a message into a thread you are a member of and returns hop accounting: accepted, seq, hop, hops_left. This is the only way to reply to a partner agent - typing in your own terminal reaches nobody. Threads have a hop budget; when hops_left runs out the thread closes, so make each message count.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread": { "type": "string" },
                    "text": { "type": "string" },
                    "intent": {
                        "type": "string",
                        "enum": ["sparring", "delegation", "answer", "status"],
                        "description": "sparring: thinking together. delegation: you are asking the partner to do something. answer: you are answering their request. status: progress note."
                    }
                },
                "required": ["thread", "text", "intent"],
                "additionalProperties": false
            }
        },
        {
            "name": "card_inbox",
            "description": "Reads up to 5 unread messages from a thread you are a member of. Messages are NOT marked read on fetch: pass the returned batch_id back as ack_through on your next call to acknowledge them, otherwise the same batch is redelivered. Reply with card_say in the thread - never by typing in your terminal.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread": { "type": "string" },
                    "ack_through": { "type": "integer" }
                },
                "required": ["thread"],
                "additionalProperties": false
            }
        }
    ])
}

fn tools_list_result(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "tools": tool_definitions() }
    })
}

fn tools_call_result(
    value: &Value,
    id: Value,
    ops: &McpOps,
    session: Option<&str>,
    bearer_card: Option<&str>,
) -> Value {
    let params = value.get("params");
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let empty_args = json!({});
    let arguments = params
        .and_then(|p| p.get("arguments"))
        .unwrap_or(&empty_args);

    let outcome: Result<Value, String> = match name {
        "browser_card_open" => {
            let url = arguments.get("url").and_then(Value::as_str);
            ops.browser.open(session, url).map(|r| {
                json!({
                    "card_number": r.card_number,
                    "target_id": r.target_id,
                    "cdp_endpoint": r.cdp_endpoint
                })
            })
        }
        "browser_card_close" => match required_card_number(arguments) {
            Ok(n) => ops.browser.close(session, n).map(|_| json!({ "ok": true })),
            Err(e) => Err(e),
        },
        "browser_card_list" => ops.browser.list().map(|rows| {
            Value::Array(
                rows.into_iter()
                    .map(|r| {
                        json!({
                            "number": r.number,
                            "opened_by": r.opened_by,
                            "url": r.url,
                            "title": r.title,
                            "target_id": r.target_id
                        })
                    })
                    .collect(),
            )
        }),
        "browser_card_focus" => match required_card_number(arguments) {
            Ok(n) => ops.browser.focus(n).map(|_| json!({ "ok": true })),
            Err(e) => Err(e),
        },
        "card_pair" | "card_say" | "card_inbox" => match bearer_card {
            // Ingen Bearer => ingen identitet. Over HTTP naar en saadan request
            // ikke laengere hertil — identitets-gaten i `handle_request` afviste
            // den med -32600 — men armen bliver staaende som lagets EGET vaern:
            // `tools_call_result` er en ren funktion, som en fremtidig kalder
            // (eller en test) kan naa uden om gaten. Formen er en TOOL-fejl, for
            // det er den eneste form en agent kan LAESE og rette sig efter.
            None => Err("card identity required: these tools authenticate with \
                         Authorization: Bearer, not with x-talminal-session"
                .to_string()),
            Some(card) => match name {
                "card_say" => match (
                    string_arg(arguments, "thread"),
                    string_arg(arguments, "text"),
                ) {
                    (Ok(thread), Ok(text)) => ops.threads.say(
                        card,
                        thread,
                        text,
                        arguments
                            .get("intent")
                            .and_then(Value::as_str)
                            .unwrap_or("sparring"),
                    ),
                    (Err(e), _) | (_, Err(e)) => Err(e),
                },
                "card_inbox" => match string_arg(arguments, "thread") {
                    Ok(thread) => ops.threads.inbox(
                        card,
                        thread,
                        arguments.get("ack_through").and_then(Value::as_u64),
                    ),
                    Err(e) => Err(e),
                },
                // card_pair: `opening_message` er navnet paa wiren (spec §3.5).
                _ => match (
                    string_arg(arguments, "agent"),
                    string_arg(arguments, "purpose"),
                    string_arg(arguments, "opening_message"),
                ) {
                    (Ok(agent), Ok(purpose), Ok(opening)) => {
                        ops.threads.pair(card, agent, purpose, opening)
                    }
                    (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
                },
            },
        },
        _ => Err(format!("unknown tool: {name}")),
    };

    let result = match outcome {
        // MCP TextContent.text is always a string; a successful tool result
        // is JSON-serialized into that string, not embedded as a raw value.
        Ok(value) => json!({ "content": [{ "type": "text", "text": value.to_string() }] }),
        Err(message) => json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true
        }),
    };

    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn required_card_number(arguments: &Value) -> Result<u32, String> {
    arguments
        .get("card_number")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| "card_number is required".to_string())
}

fn string_arg<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{key} is required and must be a non-empty string"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Naavne/tokens er unikke pr. test (delt global CARD_TOKENS-registry,
    // tests koerer parallelt) — hver test rydder op efter sig selv.

    #[test]
    fn bearer_token_resolves_to_card_after_set_card_token() {
        set_card_token("card-7", "tok-abc");
        assert_eq!(
            session_from_headers(None, Some("Bearer tok-abc")),
            SessionLookup::Card("card-7".to_string())
        );
        clear_card_token("card-7");
    }

    /// Udledningen skelner stadig "ingen identitet" fra "afvist paastand" —
    /// beskederne er forskellige. At `Anonymous` derefter afvises af
    /// request-laget paastaas paa wiren nedenfor.
    #[test]
    fn no_identity_headers_is_anonymous() {
        assert_eq!(session_from_headers(None, None), SessionLookup::Anonymous);
    }

    #[test]
    fn unknown_bearer_token_is_invalid_never_anonymous() {
        // B4: et ukendt token maa ALDRIG degradere til Anonymous.
        assert_eq!(
            session_from_headers(None, Some("Bearer helt-ukendt-token-xyz")),
            SessionLookup::InvalidToken
        );
    }

    #[test]
    fn an_authorization_header_that_is_not_a_bearer_is_invalid_never_anonymous() {
        // Samme B4-regel, men paa PARSE-fejlen: "Basic ..." faldt foer gennem
        // `strip_prefix` til None og landede — uden session-header — i den
        // PRIVILEGEREDE Anonymous.
        for header in ["Basic aGVq", "Bearer", "Token tok-abc"] {
            let lookup = session_from_headers(None, Some(header));
            assert_eq!(lookup, SessionLookup::InvalidToken, "{header}");
            assert_ne!(lookup, SessionLookup::Anonymous, "{header}");
        }
    }

    /// Afloeser `session_header_takes_precedence_over_bearer`, som cementerede
    /// den gamle adfaerd: headeren vandt og udpegede kortet helt alene.
    #[test]
    fn session_header_no_longer_takes_precedence_over_bearer() {
        set_card_token("card-9", "tok-precedence");
        let lookup = session_from_headers(Some("card-legacy"), Some("Bearer tok-precedence"));
        assert_eq!(lookup, SessionLookup::UnverifiedSession);
        assert_ne!(
            lookup,
            SessionLookup::Card("card-legacy".to_string()),
            "en uverificeret header maa ikke kunne overtrumfe tokenet"
        );
        clear_card_token("card-9");
    }

    /// CC-vejen: `worker_mcp::write_config` skriver BEGGE headere, og main.rs
    /// registrerer tokenet under samme kortnavn — de matcher altid.
    #[test]
    fn a_matching_session_header_confirms_the_bearer_identity() {
        set_card_token("card-cc", "tok-cc");
        assert_eq!(
            session_from_headers(Some("card-cc"), Some("Bearer tok-cc")),
            SessionLookup::Card("card-cc".to_string())
        );
        clear_card_token("card-cc");
    }

    #[test]
    fn a_session_header_without_a_bearer_is_never_an_identity() {
        let lookup = session_from_headers(Some("card-uden-token"), None);
        assert_eq!(lookup, SessionLookup::UnverifiedSession);
        // Hverken den paastaaede identitet ELLER den privilegerede anonyme
        // session — en afvist paastand maa ikke forfremmes.
        assert_ne!(lookup, SessionLookup::Card("card-uden-token".to_string()));
        assert_ne!(lookup, SessionLookup::Anonymous);
    }

    #[test]
    fn set_card_token_replaces_and_clear_invalidates_both() {
        set_card_token("card-lifecycle", "t1-lifecycle");
        set_card_token("card-lifecycle", "t2-lifecycle");
        // t1 er nu ugyldig (erstattet), t2 er gyldig.
        assert_eq!(
            session_from_headers(None, Some("Bearer t1-lifecycle")),
            SessionLookup::InvalidToken
        );
        assert_eq!(
            session_from_headers(None, Some("Bearer t2-lifecycle")),
            SessionLookup::Card("card-lifecycle".to_string())
        );
        clear_card_token("card-lifecycle");
        // Efter clear er begge ugyldige.
        assert_eq!(
            session_from_headers(None, Some("Bearer t1-lifecycle")),
            SessionLookup::InvalidToken
        );
        assert_eq!(
            session_from_headers(None, Some("Bearer t2-lifecycle")),
            SessionLookup::InvalidToken
        );
    }

    // -----------------------------------------------------------------------
    // Wire-niveau (H2/M10). Headerne og bodyens stoerrelse ER paastanden, saa
    // en intern seam ville ikke bevise noget — samme raa-HTTP-form som
    // tests/mcp_server.rs.
    // -----------------------------------------------------------------------

    /// Optager hvilken session ops-laget faktisk fik at se.
    #[derive(Default)]
    struct RecordingOps {
        opens: Mutex<Vec<Option<String>>>,
    }

    impl RecordingOps {
        fn opens(&self) -> Vec<Option<String>> {
            self.opens.lock().expect("opens mutex").clone()
        }
    }

    impl BrowserCardOps for RecordingOps {
        fn open(&self, session: Option<&str>, _url: Option<&str>) -> Result<OpenResult, String> {
            self.opens
                .lock()
                .expect("opens mutex")
                .push(session.map(str::to_string));
            Ok(OpenResult {
                card_number: 1,
                target_id: "T-test".to_string(),
                cdp_endpoint: "http://127.0.0.1:0".to_string(),
            })
        }
        fn close(&self, _session: Option<&str>, _card_number: u32) -> Result<(), String> {
            Ok(())
        }
        fn list(&self) -> Result<Vec<BrowserCardRow>, String> {
            Ok(vec![])
        }
        fn focus(&self, _card_number: u32) -> Result<(), String> {
            Ok(())
        }
    }

    struct NoThreads;

    impl ThreadOps for NoThreads {
        fn pair(&self, _f: &str, _a: &str, _p: &str, _o: &str) -> Result<Value, String> {
            Err("ikke brugt i denne test".to_string())
        }
        fn say(&self, _f: &str, _t: &str, _x: &str, _i: &str) -> Result<Value, String> {
            Err("ikke brugt i denne test".to_string())
        }
        fn inbox(&self, _c: &str, _t: &str, _a: Option<u64>) -> Result<Value, String> {
            Err("ikke brugt i denne test".to_string())
        }
    }

    const OPEN_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_card_open","arguments":{}}}"#;

    fn start_recording() -> (u16, Arc<RecordingOps>) {
        let ops = Arc::new(RecordingOps::default());
        let port = start_mcp_server(McpOps {
            browser: ops.clone(),
            threads: Arc::new(NoThreads),
        })
        .expect("start");
        (port, ops)
    }

    fn post_raw(port: u16, extra_headers: &str, body: &str) -> String {
        use std::io::Write as _;
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             {extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut out = String::new();
        stream.read_to_string(&mut out).expect("read");
        out
    }

    fn body_of(raw: &str) -> Value {
        let (_headers, body) = raw.split_once("\r\n\r\n").expect("http boundary");
        serde_json::from_str(body).expect("body is valid json")
    }

    #[test]
    fn a_request_carrying_origin_is_rejected_before_it_reaches_the_tools() {
        let (port, ops) = start_recording();
        let raw = post_raw(port, "Origin: https://fjendtlig.example\r\n", OPEN_BODY);
        assert!(raw.starts_with("HTTP/1.1 403"), "{raw}");
        assert!(
            ops.opens().is_empty(),
            "en fjendtlig side maa ikke kunne aabne kort blindt: {:?}",
            ops.opens()
        );
    }

    /// Afloeser `the_same_request_without_origin_still_reaches_the_anonymous_canvas_path`,
    /// som cementerede den PRIVILEGEREDE anonyme vej (ops fik `None` at se, og
    /// browser_host lader `None` aabne canvas-ejede kort og lukke
    /// `opened_by=None`-kort). Gennemsoegningen 2026-07-29 fandt ingen legitim
    /// klient paa den vej, saa den er nu lukket i selve request-laget.
    #[test]
    fn a_request_without_any_identity_never_reaches_the_tools() {
        let (port, ops) = start_recording();
        let raw = post_raw(port, "", OPEN_BODY);
        let body = body_of(&raw);
        assert_eq!(body["error"]["code"], -32600, "{body}");
        assert!(
            ops.opens().is_empty(),
            "anonym request naaede ops-laget: {:?}",
            ops.opens()
        );
    }

    /// Det bevidste valg: gaten er ENSARTET og daekker ogsaa handshaken. Begge
    /// worker-profiler saetter identiteten paa alle deres kald, saa en
    /// halv-aaben `initialize`/`tools/list` ville kun gavne den uden token.
    #[test]
    fn initialize_and_tools_list_require_an_identity_too() {
        let (port, _ops) = start_recording();
        for body in [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ] {
            let out = body_of(&post_raw(port, "", body));
            assert_eq!(out["error"]["code"], -32600, "{out}");
            assert!(out.get("result").is_none(), "{out}");
        }
    }

    /// Den bindende graense: Claude Code 2.1.220 afkorter server-instructions
    /// HAARDT ved 2048 tegn (konstanten `NU` i bundlen; debug-loggen siger
    /// "Server instructions truncated from N to 2048 chars"). Maalt 2026-07-29.
    /// Vokser teksten forbi loftet, forsvinder slutningen tavst hos hver eneste
    /// Claude-worker — derfor er den her en test og ikke en kommentar.
    #[test]
    fn server_instructions_fit_claude_codes_2048_char_truncation() {
        let len = SERVER_INSTRUCTIONS.chars().count();
        assert!(
            len <= 2048,
            "server-instructions er {len} tegn — Claude Code afkorter ved 2048, \
             saa halen ville forsvinde tavst"
        );
    }

    /// Codex laegger teksten som `description` paa namespace-toolet og anbefaler
    /// at de foerste 512 tegn kan staa alene. Wake-kontrakten er det dyreste vi
    /// har at sige, saa den skal ligge inden for det vindue — ikke bare i teksten.
    #[test]
    fn the_wake_contract_lands_inside_codex_first_512_chars() {
        let head: String = SERVER_INSTRUCTIONS.chars().take(512).collect();
        assert!(
            head.contains("silent for 1.5 seconds"),
            "wake-kontraktens operative regel faldt ud af de foerste 512 tegn: {head}"
        );
    }

    /// Feltet skal faktisk med i haandtrykket — ikke bare findes som konstant.
    #[test]
    fn initialize_carries_the_server_instructions() {
        let request = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        let out = initialize_result(&request, json!(1));
        assert_eq!(
            out["result"]["instructions"].as_str(),
            Some(SERVER_INSTRUCTIONS),
            "{out}"
        );
    }

    /// Modstykket: med et gyldigt Bearer er handshaken uaendret.
    #[test]
    fn a_verified_caller_still_gets_the_handshake_and_the_tool_list() {
        set_card_token("card-wire-handshake", "tok-wire-handshake");
        let (port, _ops) = start_recording();
        let headers = "Authorization: Bearer tok-wire-handshake\r\n";
        let init = body_of(&post_raw(
            port,
            headers,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        ));
        assert_eq!(init["result"]["serverInfo"]["name"], SERVER_NAME, "{init}");
        let list = body_of(&post_raw(
            port,
            headers,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ));
        assert!(
            list["result"]["tools"]
                .as_array()
                .is_some_and(|t| !t.is_empty()),
            "{list}"
        );
        clear_card_token("card-wire-handshake");
    }

    #[test]
    fn a_session_header_alone_cannot_open_a_card_in_another_cards_name() {
        let (port, ops) = start_recording();
        let raw = post_raw(port, "x-talminal-session: card-offer\r\n", OPEN_BODY);
        let body = body_of(&raw);
        assert_eq!(body["error"]["code"], -32600, "{body}");
        assert!(
            ops.opens().is_empty(),
            "uverificeret header naaede ops-laget: {:?}",
            ops.opens()
        );
    }

    #[test]
    fn the_cc_path_with_both_headers_still_reaches_the_ops_layer() {
        set_card_token("card-wire-cc", "tok-wire-cc");
        let (port, ops) = start_recording();
        let raw = post_raw(
            port,
            "x-talminal-session: card-wire-cc\r\nAuthorization: Bearer tok-wire-cc\r\n",
            OPEN_BODY,
        );
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        assert_eq!(ops.opens(), vec![Some("card-wire-cc".to_string())]);
        clear_card_token("card-wire-cc");
    }

    #[test]
    fn an_oversized_content_length_is_rejected_with_413() {
        use std::io::Write as _;
        let (port, ops) = start_recording();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let announced = MAX_BODY_BYTES + 1;
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             Content-Length: {announced}\r\nConnection: close\r\n\r\n{OPEN_BODY}"
        );
        stream.write_all(request.as_bytes()).expect("write");
        // Resten af den lovede body kommer aldrig. Uden loftet ville serveren
        // vente paa den — her lukker vi skrive-siden, saa testen maaler
        // afvisningen og ikke vores egen taalmodighed.
        stream
            .shutdown(std::net::Shutdown::Write)
            .expect("shutdown write");
        let mut raw = String::new();
        stream.read_to_string(&mut raw).expect("read");
        assert!(raw.starts_with("HTTP/1.1 413"), "{raw}");
        assert!(
            ops.opens().is_empty(),
            "en afvist body maa ikke naa tool-dispatchen"
        );
    }
}
