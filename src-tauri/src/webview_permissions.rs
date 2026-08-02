//! Mikrofon-permission for appens EGET webview.
//!
//! **Hvorfor den findes.** WebView2 spørger som udgangspunkt brugeren, før en
//! side må bruge mikrofonen, og HUSKER svaret i webviewets user-data-mappe.
//! Bliver prompten afvist én gang — ved et uheld, eller fordi en browser-agtig
//! dialog i et desktop-vindue er svær at tyde — fejler `getUserMedia` derefter
//! permanent, og appen har ingen flade der kan fortryde det. Brugeren står med
//! en stum mikrofon og ingen vej tilbage.
//!
//! Prompten tilføjer heller ikke noget samtykke: appen ER en stemme-app, og
//! brugeren har netop trykket taleknappen ned. Derfor svares der ja på dens
//! vegne, og samtykket ligger dér hvor det hører hjemme — i at installere og
//! bruge programmet.
//!
//! **Grænsen der IKKE må flyttes.** Handleren hænges på hoved-webviewet alene.
//! Browser-kortene er separate webviews (`browser_host.rs`) der loader vilkårligt
//! web, og de har deres egne profil-mapper (`browser::profiles_root`). En
//! fremmed side skal fortsat spørge om lov. Derfor: `main`-vinduet, og kun det.
//!
//! **Hvad den ikke kan.** Windows' egen privatlivsindstilling (Indstillinger →
//! Privatliv og sikkerhed → Mikrofon) ligger over WebView2. Er adgangen slået
//! fra dér — af brugeren eller en politik på en arbejds-PC — fejler
//! `getUserMedia` uanset hvad denne fil gør. Den vej håndteres i frontenden,
//! som klassificerer fejlen og viser brugeren hvor indstillingen bor.

use tauri::WebviewWindow;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
    COREWEBVIEW2_PERMISSION_STATE_ALLOW,
};
use webview2_com::PermissionRequestedEventHandler;
use windows::Win32::Foundation::E_POINTER;

/// Svarer ja til mikrofon-anmodninger fra `window`s webview.
///
/// Fejl her er ikke fatale for appen: uden handleren falder WebView2 tilbage
/// til sin egen prompt, som er nøjagtig dagens adfærd. Derfor rapporteres en
/// fejl til kalderen frem for at vælte opstarten.
pub fn grant_microphone(window: &WebviewWindow) -> Result<(), String> {
    let label = window.label().to_string();
    if label != "main" {
        return Err(format!(
            "mikrofon-permission gives kun til hoved-webviewet, ikke {label}"
        ));
    }

    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    window
        .with_webview(move |webview| {
            let result = unsafe {
                let controller = webview.controller();
                controller
                    .CoreWebView2()
                    .map_err(|error| format!("CoreWebView2 utilgaengelig: {error}"))
                    .and_then(|core| {
                        let mut token = Default::default();
                        core.add_PermissionRequested(
                            &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                                // Ingen args = intet at afgoere. Vi lader
                                // WebView2 om sin egen default frem for at
                                // gaette paa brugerens vegne.
                                let Some(args) = args else {
                                    return Err(E_POINTER.into());
                                };
                                let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                                args.PermissionKind(&mut kind)?;
                                // KUN mikrofonen. Alt andet (kamera, placering,
                                // notifikationer, clipboard-read) falder
                                // igennem til den eksisterende adfaerd.
                                if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
                                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                                }
                                Ok(())
                            })),
                            &mut token,
                        )
                        .map_err(|error| format!("add_PermissionRequested fejlede: {error}"))
                    })
            };
            let _ = tx.send(result);
        })
        .map_err(|error| format!("with_webview fejlede: {error}"))?;

    // `with_webview` koerer closuren paa UI-traaden; uden det her ville vi
    // rapportere OK uden at vide om registreringen lykkedes.
    rx.recv()
        .map_err(|_| "webview-closuren svarede aldrig".to_string())?
}
