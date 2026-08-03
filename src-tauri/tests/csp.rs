//! Gate 1: CSP'en maa ikke drifte fra de ruter fladen faktisk kalder.
//!
//! Router og TTS taler med udbyderne fra RUST via reqwest, saa deres
//! endpoints skal IKKE staa i politikken. Kun STT-vejen aabnes af fladen
//! selv (`src/voice/stt.ts` laver `new WebSocket(...)` mod rutens endpoint),
//! og det er den kobling der kan gaa i stykker i stilhed: tilfoejer nogen en
//! STT-rute i `providers.rs` uden at roere `tauri.conf.json`, doer stemmen i
//! release-bygget uden at en eneste test faelder.
//!
//! Testen skriver bevidst IKKE mod `providers::warm_origins()`. Den funktion
//! kraever ogsaa en router-rute (altsaa praecis de origins der ikke skal i
//! politikken) og konverterer `wss://` til `https://` — den ville maale noget
//! andet end det der staar paa spil.

use talminal_canvas_lib::providers;

fn csp() -> String {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
    conf["app"]["security"]["csp"]
        .as_str()
        .expect("app.security.csp skal vaere sat — `null` er gate 1's udgangspunkt")
        .to_string()
}

fn connect_src(policy: &str) -> String {
    policy
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("connect-src"))
        .expect("politikken skal have et connect-src-direktiv")
        .to_string()
}

#[test]
fn every_stt_route_host_is_in_connect_src() {
    let policy = csp();
    let connect = connect_src(&policy);
    let mut checked = 0;
    for route in providers::STT_ROUTES {
        let url: url::Url = route
            .endpoint
            .parse()
            .expect("rute-endpoint skal vaere en URL");
        let host = url.host_str().expect("endpoint skal have en host");
        let origin = format!("{}://{}", url.scheme(), host);
        assert!(
            connect.contains(&origin),
            "STT-ruten '{}' peger paa {origin}, som ikke staar i connect-src.\n\
             Stemmen ville doe i release-bygget uden at nogen anden test faldt.\n\
             connect-src: {connect}",
            route.slug
        );
        checked += 1;
    }
    assert!(checked > 0, "der skal findes mindst én STT-rute at tjekke");
}

#[test]
fn ipc_origin_is_allowed() {
    // Den farligste fejlmode ved en for stram CSP: Tauris IPC degraderer
    // TAVST til postMessage med én console.warn, og et release-byg har ingen
    // devtools-konsol at se den i. Derfor er den her laast eksplicit.
    let connect = connect_src(&csp());
    assert!(
        connect.contains("ipc:") && connect.contains("http://ipc.localhost"),
        "IPC-originen mangler i connect-src — Tauris IPC ville degradere tavst. \
         connect-src: {connect}"
    );
}

#[test]
fn policy_locks_the_cheap_wins() {
    let policy = csp();
    for directive in [
        "default-src 'self'",
        "object-src 'none'",
        "base-uri 'self'",
        "form-action 'none'",
        "frame-ancestors 'none'",
    ] {
        assert!(
            policy.contains(directive),
            "politikken mangler `{directive}`"
        );
    }
}

#[test]
fn style_src_keeps_unsafe_inline_and_says_why() {
    // xterm 6 injicerer SELV runtime-`<style>`-elementer og kalder
    // `setAttribute("style", …)`. Det ligger i biblioteket og forsvinder ikke
    // ved at rydde op i projektets egen CSS. Fjerner nogen `'unsafe-inline'`
    // i den tro at det er en stramning, bliver terminalen ulaeselig — farver,
    // font og celle-hoejde kommer alle derfra.
    let policy = csp();
    assert!(
        policy.contains("style-src 'self' 'unsafe-inline'"),
        "style-src skal beholde 'unsafe-inline' — xterm injicerer egne style-elementer"
    );
}
