//! Keyring-secrets — aegte Windows Credential Manager, BYOK-noegler (Task 14).
//!
//! Backend-verdict (bekraeftet mod keyring 3.6.3-kilden): med feature
//! `windows-native` er default-storen paa Windows `windows.rs` — direkte
//! `CredWriteW`/`CredReadW`/`CredDeleteW` (generic credentials,
//! `CRED_PERSIST_ENTERPRISE`), dvs. AEGTE persistens paa tvaers af
//! processer/genstarter. UDEN featuren ville keyring falde tilbage til
//! mock-storen (in-memory) — Cargo.toml-linjen fra Task 1 er altsaa korrekt.
//!
//! Kontrakt (bindende, plan Task 14):
//! - service-navnet er `"Talminal"` (Credential Manager-entry:
//!   "Generic Credentials" med noeglenavnet som bruger).
//! - noeglenavne er bindende strenge: `"stt_api_key"` og `"router_api_key"`
//!   (konstanterne nedenfor er den kanoniske stavning — frontenden bruger
//!   de samme literaler via invoke).
//! - `load_secret` af en ukendt noegle er `Ok(None)` — IKKE en fejl
//!   (Settings-panelets "ikke sat"-tilstand).
//! - `delete_secret` er idempotent: allerede-slettet noegle => `Ok(())`.
//!
//! Noegle-VAERDIER krydser ALDRIG WebView-graensen. `load_secret`-commanden
//! mapper Rust-vaerdien til `true`/`null`; Settings ser kun sat/ikke-sat.

use std::sync::OnceLock;

use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Delt HTTP-klient: reqwest pooler keep-alive-forbindelser pr. origin
/// (idle-timeout 90 s) — en klient PER KALD ville betale TCP+TLS-håndtryk
/// på hvert eneste router-/TTS-kald (~200-400 ms målt), fordi pipeline-kald
/// ligger sekunder fra hinanden.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Varm forbindelserne til router-gatewayen og OpenAI (STT-mint/TTS) op.
/// Kaldes fire-and-forget ved PTT-tryk: mens brugeren taler, håndtrykkes
/// TLS, så router-/TTS-kaldene efter slip rammer en varm pool. Fejl
/// ignoreres bevidst — opvarmning er best-effort og må aldrig blokere.
pub async fn warm_voice_connections() {
    let settings = crate::workspace::load_settings();
    let Some(stt) = crate::providers::stt_route(&settings.stt_provider) else {
        return;
    };
    let Some(router) = crate::providers::router_route(&settings.routing_provider) else {
        return;
    };
    let mut handles = Vec::new();
    for origin in crate::providers::warm_origins(stt, router) {
        let client = http_client().clone();
        handles.push(tauri::async_runtime::spawn(async move {
            let _ = client.head(&origin).send().await;
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }
}

const REALTIME_CLIENT_SECRET_ENDPOINT: &str = "https://api.openai.com/v1/realtime/client_secrets";
const REALTIME_MODEL: &str = "gpt-realtime-2.1-mini";
const TTS_SPEECH_ENDPOINT: &str = "https://api.openai.com/v1/audio/speech";
const TTS_MODEL: &str = "gpt-4o-mini-tts";
const ROUTER_REQUEST_MAX_BYTES: usize = 16 * 1024;
const ROUTER_RESPONSE_MAX_BYTES: usize = 64 * 1024;
const TTS_RESPONSE_MAX_BYTES: usize = 4 * 1024 * 1024;
const TTS_INPUT_MAX_CHARS: usize = 600;
const TTS_VOICES: [&str; 4] = ["nova", "coral", "marin", "ash"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RealtimeClientSecret {
    pub value: String,
    pub expires_at: u64,
}

#[derive(Serialize)]
struct ClientSecretRequest<'a> {
    expires_after: ClientSecretExpiry,
    session: ClientSecretSession<'a>,
}

#[derive(Serialize)]
struct ClientSecretExpiry {
    anchor: &'static str,
    seconds: u32,
}

#[derive(Serialize)]
struct ClientSecretSession<'a> {
    r#type: &'static str,
    model: &'a str,
}

#[derive(Serialize)]
struct TranscriptionSecretRequest {
    expires_after: ClientSecretExpiry,
    session: TranscriptionSecretSession,
}

#[derive(Serialize)]
struct TranscriptionSecretSession {
    r#type: &'static str,
}

pub async fn mint_realtime_secret_with(
    api_key: &str,
    endpoint: &str,
) -> Result<RealtimeClientSecret, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("OpenAI API key is not configured".to_string());
    }
    let response = http_client()
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&ClientSecretRequest {
            expires_after: ClientSecretExpiry {
                anchor: "created_at",
                seconds: 600,
            },
            session: ClientSecretSession {
                r#type: "realtime",
                model: REALTIME_MODEL,
            },
        })
        .send()
        .await
        .map_err(|error| format!("mint realtime secret request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("mint realtime secret failed with HTTP {status}"));
    }
    response
        .json::<RealtimeClientSecret>()
        .await
        .map_err(|error| format!("mint realtime secret returned invalid JSON: {error}"))
}

pub async fn mint_realtime_secret() -> Result<RealtimeClientSecret, String> {
    // Realtime-motoren er ude af appen (spec §0), men funktionen bliver til en
    // eventuel engelsk port — og skal da laese den samme migrerede slot som
    // resten, ikke en slot migrationen har slettet.
    let api_key = stt_api_key_from(|slot| load_secret(slot.to_string()))?;
    mint_realtime_secret_with(&api_key, REALTIME_CLIENT_SECRET_ENDPOINT).await
}

fn error_excerpt(bytes: &[u8], api_key: &str) -> String {
    // Redaktion sker FOER afkortning: en noegle der krydser 500-tegns-graensen
    // ville ellers ikke laengere findes som soegbar delstreng og laekke sine
    // tegn ind i Err-strengen (ultra-verifikator-BLOCKER, boelge A).
    let full = String::from_utf8_lossy(bytes).replace(api_key, "[redacted]");
    let mut excerpt: String = full.chars().take(500).collect();
    // En noegle klippet af laese-cappen kan overleve som PRAEFIKS-suffiks paa
    // excerpten (multibyte-bodies kan naa cappen inden 500 tegn) — strip den,
    // saa intet sammenhaengende noeglemateriale (>= 4 tegn) krydser graensen.
    if api_key.len() >= 4 {
        let max = api_key.len().min(excerpt.len());
        for n in (4..=max).rev() {
            if api_key.is_char_boundary(n) && excerpt.ends_with(&api_key[..n]) {
                let cut = excerpt.len() - n;
                excerpt.truncate(cut);
                excerpt.push_str("[redacted]");
                break;
            }
        }
    }
    excerpt
}

async fn response_bytes_with_cap(
    mut response: reqwest::Response,
    operation: &str,
    max_bytes: usize,
    cap_label: &str,
    api_key: &str,
) -> Result<Vec<u8>, String> {
    let status = response.status();
    if !status.is_success() {
        // Four bytes per Unicode scalar are sufficient to recover the first
        // 500 characters without buffering an unbounded upstream error body.
        const ERROR_EXCERPT_MAX_BYTES: usize = 2_000;
        let mut bytes = Vec::with_capacity(ERROR_EXCERPT_MAX_BYTES);
        while bytes.len() < ERROR_EXCERPT_MAX_BYTES {
            let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| format!("{operation} response read failed: {error}"))?
            else {
                break;
            };
            let remaining = ERROR_EXCERPT_MAX_BYTES - bytes.len();
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        }
        return Err(format!(
            "{operation} failed with HTTP {status}: {}",
            error_excerpt(&bytes, api_key)
        ));
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("{operation} response read failed: {error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(format!("{operation} response exceeds {cap_label}"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn parse_json_object(body: &str, operation: &str) -> Result<Map<String, Value>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| format!("{operation} body must be a JSON object: {error}"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{operation} body must be a JSON object"))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    operation: &str,
) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{operation} body.{field} must be a string"))
}

pub async fn mint_transcription_secret_with(
    api_key: &str,
    endpoint: &str,
) -> Result<RealtimeClientSecret, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("OpenAI API key is not configured".to_string());
    }
    let response = http_client()
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&TranscriptionSecretRequest {
            expires_after: ClientSecretExpiry {
                anchor: "created_at",
                seconds: 600,
            },
            session: TranscriptionSecretSession {
                r#type: "transcription",
            },
        })
        .send()
        .await
        .map_err(|error| format!("mint transcription secret request failed: {error}"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("mint transcription secret response read failed: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "mint transcription secret failed with HTTP {status}: {}",
            error_excerpt(&bytes, api_key)
        ));
    }
    serde_json::from_slice::<RealtimeClientSecret>(&bytes)
        .map_err(|error| format!("mint transcription secret returned invalid JSON: {error}"))
}

/// Testbar kerne: hvilken slot STT-vejen slaar op i. Parametriseret af samme
/// grund som `resolve_router_route_with` — `SERVICE` har ingen env-override,
/// saa en test mod produktionsvejen ville roere ejerens egne noegler.
pub fn stt_api_key_from(
    load_key: impl Fn(&str) -> Result<Option<String>, String>,
) -> Result<String, String> {
    load_key(crate::providers::KEY_SLOT_OPENAI)?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| "OpenAI API key is not configured".to_string())
}

pub async fn mint_transcription_secret() -> Result<RealtimeClientSecret, String> {
    // MAA laese den MIGREREDE slot. Denne funktion er den levende STT-vej
    // (stt.ts kalder mint_transcription_secret ved hvert PTT-tryk), og
    // migrationen sletter `stt_api_key` — laeses den gamle slot her, fejler
    // al voice med "OpenAI API key is not configured" efter foerste opstart.
    let api_key = stt_api_key_from(|slot| load_secret(slot.to_string()))?;
    mint_transcription_secret_with(&api_key, REALTIME_CLIENT_SECRET_ENDPOINT).await
}

async fn send_router_request(
    api_key: &str,
    endpoint: &str,
    body: String,
) -> Result<String, String> {
    let response = http_client()
        .post(endpoint)
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|error| format!("router chat completion request failed: {error}"))?;
    let bytes = response_bytes_with_cap(
        response,
        "router chat completion",
        ROUTER_RESPONSE_MAX_BYTES,
        "64 KiB",
        api_key,
    )
    .await?;
    String::from_utf8(bytes)
        .map_err(|error| format!("router chat completion returned invalid UTF-8: {error}"))
}

/// Staggered hedge: gateway-/model-halen viser sporadiske 2-6 s-kald selv paa
/// varme forbindelser (audio-eval 2026-07-19: p95 4,4-5,8 s af enkelt-spikes,
/// mens 90 varme tekst-eval-kald toppede ved 1,6 s). Andet identiske skud
/// affyres foerst naar det foerste har haengt i dette interval; klassifikation
/// er idempotent, saa dobbelt-affyring er semantisk harmloes og koster kun
/// et ekstra billigt kald i halen.
const ROUTER_HEDGE_DELAY_MS: u64 = 1_200;

#[derive(Debug)]
pub struct ResolvedRoute {
    pub route: &'static crate::providers::RouterRoute,
    pub api_key: String,
}

pub fn resolve_router_route_with(
    routing_provider: &str,
    load_key: impl Fn(&str) -> Result<Option<String>, String>,
) -> Result<ResolvedRoute, String> {
    let route = crate::providers::router_route(routing_provider)
        .ok_or_else(|| format!("routing_provider {routing_provider:?} er ukendt"))?;
    let api_key = load_key(route.key_slot)?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| format!("Ingen API-noegle for {} — saet den i Settings", route.label))?;
    Ok(ResolvedRoute { route, api_key })
}

pub fn resolve_router_route() -> Result<ResolvedRoute, String> {
    let settings = crate::workspace::load_settings();
    resolve_router_route_with(&settings.routing_provider, |slot| {
        load_secret(slot.to_string())
    })
}

const ROUTER_PROBE_SYSTEM_PROMPT: &str =
    "Route the Danish voice command through route_voice_intent. Never invent card numbers.";
/// Probe-skemaet SKAL være formet som routerens eget (`src/voice/router.ts`s
/// `COMMAND_PARAMETERS`) — nullable felter som `anyOf`-grene, ikke som
/// union-typer `["array","null"]`.
///
/// Det er ikke en stilistisk præference. Vercel AI Gateway oversætter
/// OpenAI-skemaet til Googles `functionDeclaration`-format, og en union-type
/// bliver til `anyOf` — men et SIDEORDNET felt som `items` bliver liggende ved
/// siden af. Vertex afviser det: *"specified other fields alongside any_of.
/// When using any_of, it must be the only field set."* `cards` var det eneste
/// felt med både union-type og et sidefelt, og derfor det eneste der fældede
/// hele proben (målt 2026-08-02: "Test forbindelsen" gav HTTP 400 på
/// Vercel-ruten, mens den RIGTIGE router-vej var grøn 41/41 over samme rute).
///
/// Den egentlige defekt var at proben testede en ANDEN kontrakt end
/// produktionen sender. En probe der ikke spejler routeren, måler ikke det den
/// påstår — den kan både fejle på noget produktionen klarer, og bestå på noget
/// produktionen falder over. Hold de to i takt.
const ROUTER_PROBE_TOOL_JSON: &str = r#"{
  "type":"function",
  "function":{
    "name":"route_voice_intent",
    "strict":true,
    "description":"Return the ordered voice commands.",
    "parameters":{
      "type":"object",
      "additionalProperties":false,
      "required":["commands","confidence","reason"],
      "properties":{
        "commands":{"type":"array","minItems":1,"maxItems":10,"items":{
          "type":"object","additionalProperties":false,
          "required":["kind","card","text","cards","all","count","url_hint","agent"],
          "properties":{
            "kind":{"type":"string","enum":["send_prompt","new_card","close_cards","restart_card","open_browser","reject"]},
            "card":{"anyOf":[{"type":"integer","minimum":1},{"type":"null"}]},
            "text":{"anyOf":[{"type":"string"},{"type":"null"}]},
            "cards":{"anyOf":[{"type":"array","items":{"type":"integer","minimum":1}},{"type":"null"}]},
            "all":{"anyOf":[{"type":"boolean"},{"type":"null"}]},
            "count":{"anyOf":[{"type":"integer","minimum":1,"maximum":10},{"type":"null"}]},
            "url_hint":{"anyOf":[{"type":"string","enum":["github","google"]},{"type":"null"}]},
            "agent":{"anyOf":[{"type":"string","enum":["claude","codex"]},{"type":"null"}]}
          }
        }},
        "confidence":{"type":"number","minimum":0,"maximum":1},
        "reason":{"anyOf":[{"type":"string"},{"type":"null"}]}
      }
    }
  }
}"#;

pub(crate) fn probe_response_is_close_cards_2_3(response: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(response) else {
        return false;
    };
    let Some(arguments) = value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("tool_calls"))
        .and_then(|t| t.get(0))
        .and_then(|t| t.get("function"))
        .and_then(|f| f.get("arguments"))
        .and_then(Value::as_str)
    else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(arguments) else {
        return false;
    };
    let Some(commands) = parsed.get("commands").and_then(Value::as_array) else {
        return false;
    };
    if commands.len() != 1 {
        return false;
    }
    let command = &commands[0];
    command.get("kind").and_then(Value::as_str) == Some("close_cards")
        && command
            .get("cards")
            .and_then(Value::as_array)
            .map(|cards| cards.iter().filter_map(Value::as_i64).collect::<Vec<_>>())
            == Some(vec![2, 3])
}

pub async fn probe_router_route() -> Result<String, String> {
    let resolved = resolve_router_route()?;
    let label = resolved.route.label.to_string();
    let endpoint = resolved.route.endpoint;
    let body = serde_json::json!({
        "max_completion_tokens": 1024,
        "temperature": 0,
        "messages": [
            { "role": "system", "content": ROUTER_PROBE_SYSTEM_PROMPT },
            { "role": "user", "content": "Luk kort to og tre." }
        ],
        "tools": [serde_json::from_str::<Value>(ROUTER_PROBE_TOOL_JSON)
            .map_err(|error| format!("probe tool schema is invalid: {error}"))?],
        "tool_choice": { "type": "function", "function": { "name": "route_voice_intent" } },
        "parallel_tool_calls": false
    })
    .to_string();
    let response = router_chat_completion_with(&resolved, endpoint, body).await?;
    if probe_response_is_close_cards_2_3(&response) {
        Ok(format!("{label} forstod prøve-ytringen"))
    } else {
        Err(format!(
            "{label} svarede, men forstod ikke prøve-ytringen — strict function-calling fejlede"
        ))
    }
}

pub async fn router_chat_completion_with(
    resolved: &ResolvedRoute,
    endpoint: &str,
    body: String,
) -> Result<String, String> {
    if body.len() > ROUTER_REQUEST_MAX_BYTES {
        return Err("router chat completion body exceeds 16 KiB".to_string());
    }
    let mut object = parse_json_object(&body, "router chat completion")?;
    if object.contains_key("model") {
        return Err("router chat completion body must not carry model".to_string());
    }
    object.insert(
        "model".to_string(),
        Value::String(resolved.route.model.to_string()),
    );
    // Udtoemmende match UDEN catch-all: en ny Decoration-variant skal fejle ved
    // kompilering her, ikke blive tavst udeladt af request-body'en.
    match resolved.route.decoration {
        crate::providers::Decoration::None => {}
        crate::providers::Decoration::VercelGateway => {
            object.insert(
                "providerOptions".to_string(),
                serde_json::json!({ "gateway": { "sort": "ttft" } }),
            );
        }
        crate::providers::Decoration::ReasoningOff => {
            object.insert(
                "reasoning_effort".to_string(),
                Value::String("none".to_string()),
            );
        }
    }
    let body = serde_json::to_string(&Value::Object(object))
        .map_err(|error| format!("router chat completion body serialize failed: {error}"))?;
    let api_key = resolved.api_key.trim();
    if api_key.is_empty() {
        return Err("router API key is not configured".to_string());
    }

    if !resolved.route.hedge {
        return send_router_request(api_key, endpoint, body).await;
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<String, String>>(2);
    {
        let tx = tx.clone();
        let api_key = api_key.to_string();
        let endpoint = endpoint.to_string();
        let body = body.clone();
        tauri::async_runtime::spawn(async move {
            let _ = tx
                .send(send_router_request(&api_key, &endpoint, body).await)
                .await;
        });
    }
    {
        let api_key = api_key.to_string();
        let endpoint = endpoint.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ROUTER_HEDGE_DELAY_MS)).await;
            // Foerste svar er allerede leveret og modtageren vaek -> ingen
            // hedge (is_closed = rx dropped). Ellers affyres skud nr. 2.
            if tx.is_closed() {
                return;
            }
            let _ = tx
                .send(send_router_request(&api_key, &endpoint, body).await)
                .await;
        });
    }

    let first = rx
        .recv()
        .await
        .ok_or_else(|| "router hedge channel closed unexpectedly".to_string())?;
    if first.is_ok() {
        return first;
    }
    // Foerste fuldfoerte forsoeg fejlede — afvent hedgen som sidste chance;
    // fejler den ogsaa (eller udeblev den), rapporteres den FOERSTE fejl.
    match rx.recv().await {
        Some(second) if second.is_ok() => second,
        _ => first,
    }
}

pub async fn router_chat_completion(body: String) -> Result<String, String> {
    let resolved = resolve_router_route()?;
    let endpoint = resolved.route.endpoint;
    router_chat_completion_with(&resolved, endpoint, body).await
}

pub async fn tts_speech_with(
    api_key: &str,
    endpoint: &str,
    body: String,
) -> Result<String, String> {
    let object = parse_json_object(&body, "TTS speech")?;
    let model = required_string(&object, "model", "TTS speech")?;
    if model != TTS_MODEL {
        return Err(format!("TTS speech body.model must be {TTS_MODEL}"));
    }
    let voice = required_string(&object, "voice", "TTS speech")?;
    if !TTS_VOICES.contains(&voice) {
        return Err(format!(
            "TTS speech body.voice must be one of {}",
            TTS_VOICES.join(", ")
        ));
    }
    let response_format = required_string(&object, "response_format", "TTS speech")?;
    if response_format != "pcm" {
        return Err("TTS speech body.response_format must be pcm".to_string());
    }
    let input = required_string(&object, "input", "TTS speech")?;
    if input.chars().count() > TTS_INPUT_MAX_CHARS {
        return Err("TTS speech body.input exceeds 600 characters".to_string());
    }
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("OpenAI API key is not configured".to_string());
    }
    let response = http_client()
        .post(endpoint)
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|error| format!("TTS speech request failed: {error}"))?;
    let bytes = response_bytes_with_cap(
        response,
        "TTS speech",
        TTS_RESPONSE_MAX_BYTES,
        "4 MiB",
        api_key,
    )
    .await?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        bytes,
    ))
}

pub async fn tts_speech(body: String) -> Result<String, String> {
    let api_key = load_secret(crate::providers::KEY_SLOT_OPENAI.to_string())?
        .ok_or_else(|| "OpenAI API key is not configured".to_string())?;
    tts_speech_with(&api_key, TTS_SPEECH_ENDPOINT, body).await
}

pub async fn tts_speech_stream_with<F: FnMut(String)>(
    api_key: &str,
    endpoint: &str,
    body: String,
    mut on_chunk: F,
) -> Result<(), String> {
    let object = parse_json_object(&body, "TTS speech")?;
    let model = required_string(&object, "model", "TTS speech")?;
    if model != TTS_MODEL {
        return Err(format!("TTS speech body.model must be {TTS_MODEL}"));
    }
    let voice = required_string(&object, "voice", "TTS speech")?;
    if !TTS_VOICES.contains(&voice) {
        return Err(format!(
            "TTS speech body.voice must be one of {}",
            TTS_VOICES.join(", ")
        ));
    }
    let response_format = required_string(&object, "response_format", "TTS speech")?;
    if response_format != "pcm" {
        return Err("TTS speech body.response_format must be pcm".to_string());
    }
    let input = required_string(&object, "input", "TTS speech")?;
    if input.chars().count() > TTS_INPUT_MAX_CHARS {
        return Err("TTS speech body.input exceeds 600 characters".to_string());
    }
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("OpenAI API key is not configured".to_string());
    }
    let mut response = http_client()
        .post(endpoint)
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|error| format!("TTS speech request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        const ERROR_EXCERPT_MAX_BYTES: usize = 2_000;
        let mut bytes = Vec::with_capacity(ERROR_EXCERPT_MAX_BYTES);
        while bytes.len() < ERROR_EXCERPT_MAX_BYTES {
            let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| format!("TTS speech response read failed: {error}"))?
            else {
                break;
            };
            let remaining = ERROR_EXCERPT_MAX_BYTES - bytes.len();
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        }
        return Err(format!(
            "TTS speech failed with HTTP {status}: {}",
            error_excerpt(&bytes, api_key)
        ));
    }

    // PCM er 16-bit little-endian; HTTP-chunkgraenser er vilkaarlige, saa en
    // ulige hale skal baeres over til naeste chunk — ellers forskydes samples
    // og afspilningen bliver stoej (WebView-playeren dropper skaeve bytes).
    let mut total: usize = 0;
    let mut carry: Vec<u8> = Vec::with_capacity(1);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("TTS speech response read failed: {error}"))?
    {
        if total.saturating_add(chunk.len()) > TTS_RESPONSE_MAX_BYTES {
            return Err("TTS speech response exceeds 4 MiB".to_string());
        }
        total += chunk.len();
        carry.extend_from_slice(&chunk);
        let aligned = carry.len() & !1;
        if aligned == 0 {
            continue;
        }
        let rest = carry.split_off(aligned);
        let emit = std::mem::replace(&mut carry, rest);
        on_chunk(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            emit,
        ));
    }
    Ok(())
}

pub async fn tts_speech_stream<F: FnMut(String)>(body: String, on_chunk: F) -> Result<(), String> {
    let api_key = load_secret(crate::providers::KEY_SLOT_OPENAI.to_string())?
        .ok_or_else(|| "OpenAI API key is not configured".to_string())?;
    tts_speech_stream_with(&api_key, TTS_SPEECH_ENDPOINT, body, on_chunk).await
}

/// Credential Manager-service (bindende streng fra planens interface).
pub const SERVICE: &str = "Talminal";

/// Bindende noeglenavne (kanonisk stavning — brug disse, aldrig frie strenge).
pub const KEY_STT_API_KEY: &str = "stt_api_key";
pub const KEY_ROUTER_API_KEY: &str = "router_api_key";
pub const KEY_GATEWAY_API_KEY: &str = "gateway_api_key";

fn entry(key: &str) -> Result<Entry, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("secret key must be non-empty".to_string());
    }
    Entry::new(SERVICE, key).map_err(|e| format!("keyring entry for '{key}' failed: {e}"))
}

pub fn store_secret(key: String, value: String) -> Result<(), String> {
    entry(&key)?
        .set_password(&value)
        .map_err(|e| format!("store_secret '{key}' failed: {e}"))
}

pub fn load_secret(key: String) -> Result<Option<String>, String> {
    match entry(&key)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(format!("load_secret '{key}' failed: {e}")),
    }
}

pub fn secret_presence(value: Option<String>) -> Option<bool> {
    value.map(|_| true)
}

pub fn delete_secret(key: String) -> Result<(), String> {
    match entry(&key)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(e) => Err(format!("delete_secret '{key}' failed: {e}")),
    }
}

/// Flyt noegler til nye slots. Raekkefoelgen er BINDENDE:
/// kopier alle -> verificer alle -> slet foerst derefter.
///
/// Slot-navnene er parametre, fordi keyring-servicen ikke har nogen
/// env-override. Test maa derfor aldrig ramme produktionsnavnene.
pub fn migrate_key_slots(moves: &[(&str, &str)], drop: &[&str]) -> Result<(), String> {
    let mut copied: Vec<&str> = Vec::new();

    for (from, to) in moves {
        let Some(value) = load_secret((*from).to_string())? else {
            continue;
        };
        if load_secret((*to).to_string())?.is_some() {
            copied.push(*from);
            continue;
        }
        store_secret((*to).to_string(), value)?;
        copied.push(*from);
    }

    for (from, to) in moves {
        if !copied.contains(from) {
            continue;
        }
        if load_secret((*to).to_string())?.is_none() {
            return Err(format!(
                "migration af '{from}' til '{to}' kunne ikke verificeres — intet slettes"
            ));
        }
    }

    for from in copied {
        delete_secret(from.to_string())?;
    }
    for slot in drop {
        delete_secret((*slot).to_string())?;
    }
    Ok(())
}

/// Produktions-wrapperen med de bindende gamle og nye slot-navne.
pub fn migrate_provider_key_slots() -> Result<(), String> {
    migrate_key_slots(
        &[
            (KEY_STT_API_KEY, crate::providers::KEY_SLOT_OPENAI),
            (KEY_GATEWAY_API_KEY, crate::providers::KEY_SLOT_VERCEL),
        ],
        &[KEY_ROUTER_API_KEY],
    )
}

#[cfg(test)]
mod realtime_secret_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use serde_json::{json, Value};

    use super::{
        mint_realtime_secret_with, mint_transcription_secret_with, resolve_router_route_with,
        router_chat_completion_with, secret_presence, tts_speech_stream_with, tts_speech_with,
        ResolvedRoute, ROUTER_PROBE_TOOL_JSON,
    };

    fn resolved(slug: &str, api_key: &str) -> ResolvedRoute {
        ResolvedRoute {
            route: crate::providers::router_route(slug).expect("test route"),
            api_key: api_key.to_string(),
        }
    }

    fn mock_server(status: &str, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let address = listener.local_addr().expect("mock address");
        let status = status.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buffer = vec![0_u8; 16 * 1024];
            let read = stream.read(&mut buffer).expect("read request");
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            request
        });
        (
            format!("http://{address}/v1/realtime/client_secrets"),
            handle,
        )
    }

    fn proxy_mock_server(
        status: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind proxy mock server");
        let address = listener.local_addr().expect("proxy mock address");
        let status = status.to_string();
        let content_type = content_type.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept proxy request");
            let mut buffer = vec![0_u8; 32 * 1024];
            let read = stream.read(&mut buffer).expect("read proxy request");
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let headers = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).expect("write headers");
            // Cap-tests intentionally let the client close as soon as the
            // validated response ceiling is crossed.
            let _ = stream.write_all(&body);
            request
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn webview_presence_never_contains_the_raw_secret() {
        let presence = secret_presence(Some("sk-never-cross-webview".to_string()));
        assert_eq!(presence, Some(true));
        assert!(!serde_json::to_string(&presence)
            .expect("serialize presence")
            .contains("sk-never-cross-webview"));
        assert_eq!(secret_presence(None), None);
    }

    #[test]
    fn mint_uses_server_side_key_and_returns_only_ephemeral_secret() {
        let (endpoint, request) = mock_server(
            "200 OK",
            r#"{"value":"ek-short-lived","expires_at":1756310470,"session":{"type":"realtime"}}"#,
        );

        let secret =
            tauri::async_runtime::block_on(mint_realtime_secret_with("sk-long-lived", &endpoint))
                .expect("mint realtime secret");
        let request = request.join().expect("mock server thread");

        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-long-lived"));
        assert!(request.contains("POST /v1/realtime/client_secrets"));
        assert!(request.contains(r#""type":"realtime""#));
        assert!(request.contains(r#""model":"gpt-realtime-2.1-mini""#));
        assert_eq!(secret.value, "ek-short-lived");
        assert_eq!(secret.expires_at, 1_756_310_470);
        assert!(!format!("{secret:?}").contains("sk-long-lived"));
    }

    #[test]
    fn mint_rejects_empty_long_lived_key_before_network() {
        let error = tauri::async_runtime::block_on(mint_realtime_secret_with(
            "  ",
            "http://127.0.0.1:9/v1/realtime/client_secrets",
        ))
        .expect_err("empty key must fail");
        assert!(error.contains("OpenAI API key"));
    }

    #[test]
    fn upstream_error_is_sanitized_and_never_leaks_long_lived_key() {
        let (endpoint, request) = mock_server("401 Unauthorized", r#"{"error":"bad key"}"#);
        let error = tauri::async_runtime::block_on(mint_realtime_secret_with(
            "sk-never-log-this",
            &endpoint,
        ))
        .expect_err("401 must fail");
        request.join().expect("mock server thread");

        assert!(error.contains("401"));
        assert!(!error.contains("sk-never-log-this"));
    }

    #[test]
    fn mint_transcription_secret_posts_transcription_session() {
        let (endpoint, request) = proxy_mock_server(
            "200 OK",
            "application/json",
            br#"{"value":"ek-transcription","expires_at":1756310470}"#.to_vec(),
        );

        let secret = tauri::async_runtime::block_on(mint_transcription_secret_with(
            "sk-stt-long-lived",
            &format!("{endpoint}/v1/realtime/client_secrets"),
        ))
        .expect("mint transcription secret");
        let request = request.join().expect("proxy mock thread");

        assert!(request.contains("POST /v1/realtime/client_secrets"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-stt-long-lived"));
        assert!(request.contains(r#""expires_after":{"anchor":"created_at","seconds":600}"#));
        assert!(request.contains(r#""session":{"type":"transcription"}"#));
        assert!(!request.contains(r#""model""#));
        assert_eq!(secret.value, "ek-transcription");
        assert_eq!(secret.expires_at, 1_756_310_470);
    }

    #[test]
    fn router_injects_model_and_decoration_from_the_route() {
        let response = r#"{"choices":[{"message":{"content":"ok"}}]}"#;
        let (endpoint, request) =
            proxy_mock_server("200 OK", "application/json", response.as_bytes().to_vec());
        let route = resolved("vercel", "sk-router-only");
        let body = r#"{"messages":[],"temperature":0}"#.to_string();

        let returned = tauri::async_runtime::block_on(router_chat_completion_with(
            &route,
            &format!("{endpoint}/v1/chat/completions"),
            body,
        ))
        .expect("router completion");
        let request = request.join().expect("proxy mock thread");

        assert!(request.contains("POST /v1/chat/completions"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-router-only"));
        assert!(request.contains(r#""model":"google/gemini-3.1-flash-lite""#));
        assert!(request.contains(r#""sort":"ttft""#));
        assert_eq!(returned, response);
    }

    /// Vertex' regel, haandhaevet paa vores side: naar `anyOf` er sat, skal det
    /// vaere det ENESTE felt i objektet — og en union-type (`"type":[a,b]`) er
    /// forbudt, fordi gatewayen oversaetter den TIL `anyOf` og efterlader
    /// sidefelter som `items` ved siden af. Det var praecis den kombination der
    /// gav HTTP 400 paa `cards` (2026-08-02).
    ///
    /// Testen gaar rekursivt, saa et nyt felt ikke kan snige den forbudte form
    /// ind et sted ingen kigger.
    #[test]
    fn probe_schema_obeys_the_any_of_rule() {
        fn walk(node: &Value, path: &str) {
            match node {
                Value::Object(map) => {
                    if let Some(kind) = map.get("type") {
                        assert!(
                            !kind.is_array(),
                            "{path}.type er en union-type; brug anyOf-grene i stedet \
                             (gatewayen oversaetter unionen til anyOf og efterlader \
                             sidefelter, hvad Vertex afviser)"
                        );
                    }
                    if map.contains_key("anyOf") {
                        let extra: Vec<&String> =
                            map.keys().filter(|key| key.as_str() != "anyOf").collect();
                        assert!(
                            extra.is_empty(),
                            "{path} saetter anyOf sammen med {extra:?}; anyOf skal staa alene"
                        );
                    }
                    for (key, value) in map {
                        walk(value, &format!("{path}.{key}"));
                    }
                }
                Value::Array(items) => {
                    for (index, value) in items.iter().enumerate() {
                        walk(value, &format!("{path}[{index}]"));
                    }
                }
                _ => {}
            }
        }

        let schema: Value = serde_json::from_str(ROUTER_PROBE_TOOL_JSON)
            .expect("probe-skemaet skal vaere gyldig JSON");
        walk(&schema, "tool");
    }

    /// Proben skal maale den kontrakt produktionen SENDER. Divergerer de to,
    /// kan proben baade fejle paa noget routeren klarer og bestaa paa noget
    /// routeren falder over — begge dele goer knappen vaerre end ingen knap.
    /// Feltnavnene her er dem `router.ts`' COMMAND_PARAMETERS kraever.
    #[test]
    fn probe_schema_matches_the_routers_command_contract() {
        let schema: Value = serde_json::from_str(ROUTER_PROBE_TOOL_JSON).expect("probe-skemaet");
        let command = &schema["function"]["parameters"]["properties"]["commands"]["items"];

        let required: Vec<&str> = command["required"]
            .as_array()
            .expect("required-listen")
            .iter()
            .map(|value| value.as_str().expect("feltnavn"))
            .collect();
        assert_eq!(
            required,
            vec!["kind", "card", "text", "cards", "all", "count", "url_hint", "agent"],
        );
        // Enum-vaerdierne er en del af kontrakten: degenererer de til bar
        // "string", holder proben op med at teste strict function-calling.
        assert_eq!(
            command["properties"]["agent"]["anyOf"][0]["enum"],
            json!(["claude", "codex"])
        );
        assert_eq!(
            command["properties"]["url_hint"]["anyOf"][0]["enum"],
            json!(["github", "google"])
        );
        assert_eq!(
            command["properties"]["cards"]["anyOf"][0]["items"]["minimum"],
            json!(1)
        );
    }

    #[test]
    fn router_omits_decoration_outside_vercel() {
        let response = r#"{"choices":[{"message":{"content":"ok"}}]}"#;
        let (endpoint, request) =
            proxy_mock_server("200 OK", "application/json", response.as_bytes().to_vec());
        let route = resolved("google", "sk-google");

        tauri::async_runtime::block_on(router_chat_completion_with(
            &route,
            &format!("{endpoint}/v1/chat/completions"),
            r#"{"messages":[]}"#.to_string(),
        ))
        .expect("router completion");
        let request = request.join().expect("proxy mock thread");

        assert!(request.contains(r#""model":"gemini-3.1-flash-lite""#));
        assert!(!request.contains("providerOptions"));
        assert!(!request.contains("ttft"));
        assert!(
            !request.contains("reasoning_effort"),
            "reasoning_effort hoerer KUN til gpt-5.6-ruten"
        );
    }

    /// GPT-5.6 afviser function tools paa chat/completions ved enhver
    /// reasoning_effort != "none". Routeren er et tvunget tool-kald, saa
    /// forsvinder feltet fra body'en, doer ruten for enhver ytring — og det
    /// ville se ud som en model-fejl, ikke som en manglende dekoration.
    /// Denne test er stedet det opdages.
    #[test]
    fn router_sends_reasoning_off_on_the_gpt5_route() {
        let response = r#"{"choices":[{"message":{"content":"ok"}}]}"#;
        let (endpoint, request) =
            proxy_mock_server("200 OK", "application/json", response.as_bytes().to_vec());
        let route = resolved("openai", "sk-openai");

        tauri::async_runtime::block_on(router_chat_completion_with(
            &route,
            &format!("{endpoint}/v1/chat/completions"),
            r#"{"messages":[]}"#.to_string(),
        ))
        .expect("router completion");
        let request = request.join().expect("proxy mock thread");

        assert!(request.contains(r#""model":"gpt-5.6-luna""#));
        assert!(request.contains(r#""reasoning_effort":"none""#));
        // Vercel-dekorationen maa ikke laekke over paa en anden rute.
        assert!(!request.contains("providerOptions"));
    }

    #[test]
    fn router_rejects_a_body_that_carries_its_own_model() {
        let route = resolved("vercel", "sk-router");
        let error = tauri::async_runtime::block_on(router_chat_completion_with(
            &route,
            "http://127.0.0.1:9/v1/chat/completions",
            r#"{"model":"gpt-4o-mini","messages":[]}"#.to_string(),
        ))
        .expect_err("body med egen model skal afvises foer netvaerk");
        assert!(error.contains("model"));
    }

    #[test]
    fn router_probe_accepts_the_expected_close_cards_result() {
        let response = r#"{"choices":[{"message":{"tool_calls":[{"type":"function","function":{"name":"route_voice_intent","arguments":"{\"commands\":[{\"kind\":\"close_cards\",\"cards\":[2,3]}],\"confidence\":0.95,\"reason\":null}"}}]}}]}"#;
        assert!(super::probe_response_is_close_cards_2_3(response));
    }

    #[test]
    fn router_probe_rejects_invalid_results() {
        assert!(!super::probe_response_is_close_cards_2_3(
            r#"{"choices":[{"message":{"content":"Luk kort to og tre."}}]}"#
        ));
        let wrong = r#"{"choices":[{"message":{"tool_calls":[{"function":{"arguments":"{\"commands\":[{\"kind\":\"close_cards\",\"cards\":[1]}]}"}}]}}]}"#;
        assert!(!super::probe_response_is_close_cards_2_3(wrong));
    }

    #[test]
    fn stt_path_reads_the_migrated_slot_not_the_deleted_one() {
        // REGRESSION: mint_transcription_secret er den LEVENDE STT-vej
        // (stt.ts kalder den ved hvert PTT-tryk). Laeser den `stt_api_key`,
        // som migrationen SLETTER, fejler al voice med
        // "OpenAI API key is not configured" efter foerste opstart — og
        // ingen af de 730 oevrige tests ser det, fordi de kalder
        // `*_with(api_key, ...)`-varianterne, der faar noeglen som argument.
        let asked = std::cell::RefCell::new(Vec::new());
        let key = super::stt_api_key_from(|slot| {
            asked.borrow_mut().push(slot.to_string());
            Ok(Some("sk-migreret".to_string()))
        })
        .expect("stt key");

        assert_eq!(key, "sk-migreret");
        assert_eq!(
            asked.into_inner(),
            vec![crate::providers::KEY_SLOT_OPENAI.to_string()],
            "STT-vejen skal slaa op i den MIGREREDE slot"
        );
    }

    #[test]
    fn no_production_path_reads_a_slot_the_migration_deletes() {
        // Invarianten bag regressionen ovenfor: en slot der er kilde eller
        // drop-maal i migrationen findes ikke laengere efter foerste opstart,
        // saa ingen produktionsvej maa slaa op i den.
        for deleted in [
            super::KEY_STT_API_KEY,
            super::KEY_GATEWAY_API_KEY,
            super::KEY_ROUTER_API_KEY,
        ] {
            assert!(super::stt_api_key_from(|slot| {
                assert_ne!(
                    slot, deleted,
                    "STT-vejen slog op i '{deleted}', som migrationen sletter"
                );
                Ok(Some("sk-test".to_string()))
            })
            .is_ok());
        }
    }

    #[test]
    fn empty_or_whitespace_stt_key_fails_closed() {
        for raw in [None, Some(String::new()), Some("   ".to_string())] {
            let error = super::stt_api_key_from(|_slot| Ok(raw.clone()))
                .expect_err("tom noegle skal fejle");
            assert!(error.contains("API key"));
        }
    }

    #[test]
    fn missing_key_fails_closed_without_touching_the_network() {
        let error = resolve_router_route_with("vercel", |_slot| Ok(None))
            .expect_err("manglende noegle skal fejle");
        assert!(error.contains("Vercel"));

        let blank = resolve_router_route_with("google", |_slot| Ok(Some("   ".to_string())))
            .expect_err("whitespace-noegle skal fejle");
        assert!(blank.contains("Google"));
    }

    #[test]
    fn resolution_pairs_each_route_with_its_own_key_slot() {
        for (slug, expected_slot) in [
            ("vercel", "provider_key_vercel"),
            ("google", "provider_key_google"),
            ("openrouter", "provider_key_openrouter"),
        ] {
            let asked = std::cell::RefCell::new(Vec::new());
            let resolved = resolve_router_route_with(slug, |slot| {
                asked.borrow_mut().push(slot.to_string());
                Ok(Some("sk-test".to_string()))
            })
            .expect("resolve");
            assert_eq!(asked.into_inner(), vec![expected_slot.to_string()]);
            assert_eq!(resolved.route.slug, slug);
        }
    }

    #[test]
    fn unknown_route_slug_fails_before_any_key_lookup() {
        let asked = std::cell::RefCell::new(false);
        let error = resolve_router_route_with("anthropic", |_slot| {
            *asked.borrow_mut() = true;
            Ok(Some("sk-test".to_string()))
        })
        .expect_err("ukendt rute skal fejle");
        assert!(error.contains("anthropic"));
        assert!(!asked.into_inner());
    }

    #[test]
    fn hedge_fires_only_on_the_vercel_route() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        fn slow_counting_server(hits: Arc<AtomicUsize>) -> String {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let address = listener.local_addr().expect("addr");
            std::thread::spawn(move || {
                for stream in listener.incoming().take(2) {
                    let hits = Arc::clone(&hits);
                    let mut stream = stream.expect("accept");
                    std::thread::spawn(move || {
                        let mut request = [0_u8; 4096];
                        let _ = stream.read(&mut request);
                        hits.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(1_800));
                        let body = r#"{"choices":[]}"#;
                        let _ = stream.write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        );
                    });
                }
            });
            format!("http://{address}/v1/chat/completions")
        }

        for (slug, expected_hits) in [("vercel", 2), ("google", 1)] {
            let hits = Arc::new(AtomicUsize::new(0));
            let endpoint = slow_counting_server(Arc::clone(&hits));
            let route = resolved(slug, "sk-test");
            let _ = tauri::async_runtime::block_on(router_chat_completion_with(
                &route,
                &endpoint,
                r#"{"messages":[]}"#.to_string(),
            ));
            assert_eq!(hits.load(Ordering::SeqCst), expected_hits, "rute {slug}");
        }
    }

    #[test]
    fn tts_speech_returns_base64_of_response_bytes() {
        let pcm = vec![0_u8, 1, 2, 127, 128, 255];
        let (endpoint, request) =
            proxy_mock_server("200 OK", "application/octet-stream", pcm.clone());
        let body =
            r#"{"model":"gpt-4o-mini-tts","voice":"ash","response_format":"pcm","input":"Hej"}"#
                .to_string();

        let returned = tauri::async_runtime::block_on(tts_speech_with(
            "sk-stt-only",
            &format!("{endpoint}/v1/audio/speech"),
            body.clone(),
        ))
        .expect("tts speech");
        let request = request.join().expect("proxy mock thread");

        assert!(request.contains("POST /v1/audio/speech"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-stt-only"));
        assert!(request.contains(&body));
        assert_eq!(
            returned,
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, pcm)
        );
    }

    #[test]
    fn tts_speech_stream_emits_base64_chunks_and_carries_odd_byte() {
        // 5 bytes: chunkgraensen ligger midt i et 16-bit-sample naar serveren
        // chunker vilkaarligt; streamen skal alligevel kun emitte lige laengder.
        let pcm = vec![10_u8, 11, 12, 13, 14, 15];
        let (endpoint, request) =
            proxy_mock_server("200 OK", "application/octet-stream", pcm.clone());
        let body =
            r#"{"model":"gpt-4o-mini-tts","voice":"nova","response_format":"pcm","input":"Hej"}"#
                .to_string();

        let mut chunks: Vec<String> = Vec::new();
        tauri::async_runtime::block_on(tts_speech_stream_with(
            "sk-stt-only",
            &format!("{endpoint}/v1/audio/speech"),
            body.clone(),
            |chunk| chunks.push(chunk),
        ))
        .expect("tts stream");
        let request = request.join().expect("proxy mock thread");

        assert!(request.contains("POST /v1/audio/speech"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-stt-only"));
        let decoded: Vec<u8> = chunks
            .iter()
            .flat_map(|chunk| {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, chunk)
                    .expect("valid base64 chunk")
            })
            .collect();
        assert_eq!(decoded, pcm);
        for chunk in &chunks {
            let len = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, chunk)
                .expect("valid base64 chunk")
                .len();
            assert_eq!(len % 2, 0, "stream must only emit 16-bit-aligned chunks");
        }
    }

    #[test]
    fn tts_speech_stream_validates_before_network() {
        let error = tauri::async_runtime::block_on(tts_speech_stream_with(
            "sk-stt",
            "http://127.0.0.1:9/v1/audio/speech",
            r#"{"model":"tts-1","voice":"nova","response_format":"pcm","input":"Hej"}"#.to_string(),
            |_chunk| panic!("must not emit on validation failure"),
        ))
        .expect_err("unpinned tts model must fail before network");
        assert!(error.contains("body.model must be gpt-4o-mini-tts"));
    }

    #[test]
    fn tts_speech_stream_reports_http_error_with_redaction() {
        let (endpoint, _request) = proxy_mock_server(
            "401 Unauthorized",
            "application/json",
            br#"{"error":{"message":"bad key sk-stt-only"}}"#.to_vec(),
        );
        let body =
            r#"{"model":"gpt-4o-mini-tts","voice":"nova","response_format":"pcm","input":"Hej"}"#
                .to_string();

        let error = tauri::async_runtime::block_on(tts_speech_stream_with(
            "sk-stt-only",
            &format!("{endpoint}/v1/audio/speech"),
            body,
            |_chunk| panic!("must not emit on HTTP error"),
        ))
        .expect_err("401 must be an Err");
        assert!(error.contains("HTTP 401"));
        assert!(error.contains("[redacted]"));
        assert!(!error.contains("sk-stt-only"));
    }

    #[test]
    fn proxy_rejects_non_object_body() {
        let route = resolved("vercel", "sk-router");
        let router_error = tauri::async_runtime::block_on(router_chat_completion_with(
            &route,
            "http://127.0.0.1:9/v1/chat/completions",
            "[]".to_string(),
        ))
        .expect_err("router array body must fail before network");
        let tts_error = tauri::async_runtime::block_on(tts_speech_with(
            "sk-stt",
            "http://127.0.0.1:9/v1/audio/speech",
            "null".to_string(),
        ))
        .expect_err("tts null body must fail before network");

        assert!(router_error.contains("JSON object"));
        assert!(tts_error.contains("JSON object"));
    }

    #[test]
    fn tts_rejects_unlisted_voice() {
        let error = tauri::async_runtime::block_on(tts_speech_with(
            "sk-stt",
            "http://127.0.0.1:9/v1/audio/speech",
            r#"{"model":"gpt-4o-mini-tts","voice":"alloy","response_format":"pcm","input":"Hej"}"#
                .to_string(),
        ))
        .expect_err("unlisted TTS voice must fail before network");

        assert!(error.contains("voice"));
    }

    #[test]
    fn tts_rejects_non_pcm_format() {
        let error = tauri::async_runtime::block_on(tts_speech_with(
            "sk-stt",
            "http://127.0.0.1:9/v1/audio/speech",
            r#"{"model":"gpt-4o-mini-tts","voice":"nova","response_format":"mp3","input":"Hej"}"#
                .to_string(),
        ))
        .expect_err("non-PCM TTS format must fail before network");

        assert!(error.contains("response_format"));
        assert!(error.contains("pcm"));
    }

    #[test]
    fn tts_rejects_oversized_input() {
        let body = serde_json::json!({
            "model": "gpt-4o-mini-tts",
            "voice": "nova",
            "response_format": "pcm",
            "input": "x".repeat(601),
        })
        .to_string();
        let error = tauri::async_runtime::block_on(tts_speech_with(
            "sk-stt",
            "http://127.0.0.1:9/v1/audio/speech",
            body,
        ))
        .expect_err("oversized TTS input must fail before network");

        assert!(error.contains("600"));
    }

    #[test]
    fn proxy_maps_http_error_to_err() {
        let detail = "upstream rejected proxy request".repeat(30);
        let (endpoint, request) =
            proxy_mock_server("400 Bad Request", "text/plain", detail.clone().into_bytes());
        let route = resolved("vercel", "sk-router-never-returned");
        let error = tauri::async_runtime::block_on(router_chat_completion_with(
            &route,
            &format!("{endpoint}/v1/chat/completions"),
            r#"{"messages":[]}"#.to_string(),
        ))
        .expect_err("HTTP error must map to Err");
        request.join().expect("proxy mock thread");

        assert!(error.contains("400 Bad Request"));
        assert!(error.contains(&detail[..500]));
        assert!(!error.contains(&detail[..501]));
        assert!(!error.contains("sk-router-never-returned"));
    }

    #[test]
    fn proxy_http_error_redacts_echoed_key() {
        let (endpoint, request) = proxy_mock_server(
            "401 Unauthorized",
            "text/plain",
            b"upstream echoed sk-router-secret in its error".to_vec(),
        );
        let route = resolved("vercel", "sk-router-secret");
        let error = tauri::async_runtime::block_on(router_chat_completion_with(
            &route,
            &format!("{endpoint}/v1/chat/completions"),
            r#"{"messages":[]}"#.to_string(),
        ))
        .expect_err("echoed key must be redacted from HTTP error");
        request.join().expect("proxy mock thread");

        assert!(error.contains("401 Unauthorized"));
        assert!(!error.contains("sk-router-secret"));
        assert!(error.contains("[redacted]"));
    }

    #[test]
    fn error_excerpt_redacts_key_crossing_truncation_boundary() {
        // Verifikator-repro: 53-tegns noegle startende ved tegn 491 krydser
        // 500-tegns-graensen — redaktion foer afkortning maa ikke laekke halen.
        let api_key = format!("sk-{}", "K".repeat(50));
        let body = format!("{}{}{}", "x".repeat(491), api_key, "y".repeat(200));
        let excerpt = super::error_excerpt(body.as_bytes(), &api_key);
        // Sikkerhedskontrakten: intet sammenhaengende noeglemateriale — hverken
        // hele noeglen, dens hoved eller runs af dens indre tegn. (Selve
        // [redacted]-markoeren maa gerne klippes af 500-tegns-graensen.)
        assert!(!excerpt.contains(&api_key));
        assert!(!excerpt.contains(&api_key[..8]));
        assert!(!excerpt.contains("KKKK"));
    }

    #[test]
    fn error_excerpt_strips_key_prefix_clipped_at_excerpt_end() {
        // Laese-cappen kan klippe en noegle midt over, saa kun et praefiks naar
        // excerpten — suffiks-strippen maa ikke lade det overleve.
        let api_key = format!("sk-{}", "K".repeat(600));
        let body = format!("{}{}", "x".repeat(480), &api_key[..20]);
        let excerpt = super::error_excerpt(body.as_bytes(), &api_key);
        assert!(!excerpt.contains(&api_key[..8]));
        assert!(excerpt.ends_with("[redacted]"));
    }

    #[test]
    fn router_rejects_oversized_request_body() {
        let body = serde_json::json!({
            "padding": "x".repeat(16 * 1024),
        })
        .to_string();
        let route = resolved("vercel", "sk-router");
        let error = tauri::async_runtime::block_on(router_chat_completion_with(
            &route,
            "http://127.0.0.1:9/v1/chat/completions",
            body,
        ))
        .expect_err("oversized router request must fail before network");

        assert!(error.contains("16 KiB"));
    }

    #[test]
    fn router_rejects_oversized_response_body() {
        let (endpoint, request) =
            proxy_mock_server("200 OK", "application/json", vec![b'x'; 64 * 1024 + 1]);
        let route = resolved("vercel", "sk-router");
        let error = tauri::async_runtime::block_on(router_chat_completion_with(
            &route,
            &format!("{endpoint}/v1/chat/completions"),
            r#"{"messages":[]}"#.to_string(),
        ))
        .expect_err("oversized router response must fail");
        request.join().expect("proxy mock thread");

        assert!(error.contains("64 KiB"));
    }

    #[test]
    fn tts_rejects_wrong_model() {
        let error = tauri::async_runtime::block_on(tts_speech_with(
            "sk-stt",
            "http://127.0.0.1:9/v1/audio/speech",
            r#"{"model":"tts-1","voice":"nova","response_format":"pcm","input":"Hej"}"#.to_string(),
        ))
        .expect_err("unpinned TTS model must fail before network");

        assert!(error.contains("gpt-4o-mini-tts"));
    }

    #[test]
    fn tts_rejects_oversized_response_body() {
        let (endpoint, request) = proxy_mock_server(
            "200 OK",
            "application/octet-stream",
            vec![0_u8; 4 * 1024 * 1024 + 1],
        );
        let error = tauri::async_runtime::block_on(tts_speech_with(
            "sk-stt",
            &format!("{endpoint}/v1/audio/speech"),
            r#"{"model":"gpt-4o-mini-tts","voice":"marin","response_format":"pcm","input":"Hej"}"#
                .to_string(),
        ))
        .expect_err("oversized TTS response must fail");
        request.join().expect("proxy mock thread");

        assert!(error.contains("4 MiB"));
    }
}
