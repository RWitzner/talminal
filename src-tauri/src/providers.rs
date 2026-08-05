//! Rute-tabellen: de veje appen kan tage til de to modeller den bruger.
//!
//! Kontrakt (spec 2026-07-28 §1): dette modul er den ENESTE kilde til
//! endpoints, modelnavne, dekoration og hedge-politik. TS har ingen kopi —
//! den faar den valgte rutes felter serveret via `get_workspace`.
//!
//! Hedge-politikken er en egenskab ved ruten, ikke en global konstant:
//! det dobbelte skud er maalt paa Vercel-halen, og uden for Vercel ville
//! det blot fakturere brugeren to gange pr. ytring.

use serde::Serialize;

// BATCH-TRANSPORTEN ER SLETTET (ejer-beslutning 2026-07-29). Den fandtes kun
// for OpenRouter-STT-ruten, og den rute er væk: samme model, men uden løbende
// tekst, med hele ventetiden efter man slipper taleknappen, og uden den
// domæne-ordliste der binder "kort to" til kortnummeret to. Ejerens
// begrundelse: en rute vi ikke kan levere god dansk på, skal vi ikke tilbyde.
//
// Med den beslutning stod `SttTransport` som en diskriminator med én arm, og
// hele batch-maskineriet (`secrets::stt_transcribe_batch*`, Tauri-kommandoen,
// `voice/sttBatch.ts`) var uopnåeligt. Det er fjernet frem for at ligge og se
// ud som om det var i brug. Skal en ikke-streamende STT-udbyder tilføjes
// senere, skrives transporten forfra — mod dén udbyders faktiske API.
//
// `openai-mini` (2026-08-04) genåbnede ikke det spørgsmål, og den er nu væk
// igen (2026-08-05): `gpt-transcribe` slog begge 4o-modeller på dansk og er
// billigere end den der var default, så et modelvalg havde intet at vælge
// imellem. Transporten er uændret gennem hele forløbet — det er stadig den
// samme realtime-session, og OpenRouter-beslutningen ovenfor står ved magt.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decoration {
    None,
    /// `providerOptions.gateway.sort = "ttft"` — Vercel-specifik.
    VercelGateway,
    /// `reasoning_effort = "none"` — påkrævet af GPT-5.6-familien.
    ///
    /// Ikke en tuning-knap: OpenAI afviser function tools på
    /// `/v1/chat/completions` med enhver anden værdi ("To use function tools,
    /// use /v1/responses or set reasoning_effort to 'none'"). Routeren ER et
    /// tvunget tool-kald, så uden den står ruten helt af. Målt 2026-08-02.
    ReasoningOff,
}

/// Hvilket felt modellen tager sit sproghint i.
///
/// Dette er IKKE en stilistisk detalje. OpenAIs nyere transskriberings-
/// modeller omdoebte feltet fra `language` (streng) til `languages` (array),
/// og deres egen guide siger ordret "Don't send both". Endpointet er det
/// SAMME for begge generationer, saa dialekten kan ikke udledes af ruten paa
/// anden vis end ved at staa skrevet.
///
/// Fejlen er tavs i praecis den retning man bevaeger sig. Maalt 2026-08-05 mod
/// `/v1/audio/transcriptions`:
///
///   gpt-transcribe    + `language`  => 200, feltet ignoreres i stilhed
///   gpt-4o-transcribe + `languages` => 400 "not supported for this model"
///
/// Opgraderer man modellen og glemmer feltet, faar man altsaa et transskript,
/// ingen fejl, og et sproghint der aldrig blev laest. Derfor er dette et felt
/// paa ruten og ikke en konstant i `stt.ts`: naeste rute kan ikke undgaa at
/// tage stilling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageField {
    /// `language: "da"` — whisper-1, gpt-4o-transcribe, gpt-4o-mini-transcribe.
    Singular,
    /// `languages: ["da"]` — gpt-transcribe, gpt-live-transcribe.
    Plural,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SttRoute {
    pub slug: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub model: &'static str,
    /// Falsk => HUD'en faar ingen loebende tekst, og turen skal ryddes
    /// eksplicit ved start (spec §4).
    ///
    /// Sand for ruten i dag, og for foerste gang paa grundlag af en MAALING:
    /// den raa realtime-log 2026-08-05 viser
    /// `conversation.item.input_audio_transcription.delta` foer `.completed`.
    /// Tidligere var flaget en stillingtagen — se `bbf0e09`, hvis advarsel om
    /// netop det derfor ikke laengere gaelder denne rute.
    pub supports_partials: bool,
    /// Falsk => STT_DOMAIN_PROMPT ignoreres af udbyderen; dansk-genkendelsen
    /// paa kort-numre bliver maalbart ringere. Skal maerkes i UI'et.
    pub supports_domain_prompt: bool,
    /// Dialekten for sproghintet. Se `LanguageField`.
    pub language_field: LanguageField,
    /// Falsk => brugerens keyword-liste maa IKKE sendes. De to 4o-modeller
    /// svarede **400 `Invalid request.`** paa feltet (maalt 2026-08-05), altsaa
    /// ikke "ignoreret" men afvist: en rute uden understoettelse ville braekke
    /// hver eneste ytring, ikke bare miste et hint.
    pub supports_keywords: bool,
    pub key_slot: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RouterRoute {
    pub slug: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub model: &'static str,
    pub decoration: Decoration,
    pub hedge: bool,
    pub key_slot: &'static str,
}

pub const KEY_SLOT_OPENAI: &str = "provider_key_openai";
pub const KEY_SLOT_VERCEL: &str = "provider_key_vercel";
pub const KEY_SLOT_GOOGLE: &str = "provider_key_google";
pub const KEY_SLOT_OPENROUTER: &str = "provider_key_openrouter";

pub const DEFAULT_STT_SLUG: &str = "openai";
pub const DEFAULT_ROUTER_SLUG: &str = "vercel";

/// ÉN rute. Modelvalget fra 2026-08-04 (`bbf0e09`) er rullet tilbage, ikke
/// fordi valget var forkert at tilbyde, men fordi maalingen gjorde det
/// overfloedigt: `gpt-transcribe` er paa dansk markant bedre end BEGGE de to
/// 4o-modeller og samtidig billigere end den der var default
/// ($0,0045/min mod $0,006 og $0,003). Naar én rute vinder paa baade pris og
/// praecision, er et valg mellem dem kun en maade at lade brugeren vaelge
/// forkert paa.
///
/// Tabellen er stadig en LISTE og ikke en konstant. Det er ikke spekulativ
/// generalitet: den har vaeret én rute foer (frem til 2026-08-04) og er blevet
/// til to og tilbage igen, og `resolve_voice_routes` slaar fortsat op paa slug.
///
/// Slug'en `openai` er GENBRUGT frem for at faa et nyt navn. Det goer
/// migreringen gratis: en gemt `openai` peger nu paa den nye model, og en gemt
/// `openai-mini` findes ikke laengere, saa `normalize_settings` bringer den
/// her. Prisen er at aendringen er tavs i brugerens `settings.json` — filen ser
/// uaendret ud mens modellen bag er en anden.
pub const STT_ROUTES: &[SttRoute] = &[SttRoute {
    slug: "openai",
    label: "OpenAI",
    endpoint: "wss://api.openai.com/v1/realtime?intent=transcription",
    model: "gpt-transcribe",
    supports_partials: true,
    supports_domain_prompt: true,
    language_field: LanguageField::Plural,
    supports_keywords: true,
    key_slot: KEY_SLOT_OPENAI,
}];

pub const ROUTER_ROUTES: &[RouterRoute] = &[
    RouterRoute {
        slug: "vercel",
        label: "Vercel AI Gateway",
        endpoint: "https://ai-gateway.vercel.sh/v1/chat/completions",
        model: "google/gemini-3.1-flash-lite",
        decoration: Decoration::VercelGateway,
        hedge: true,
        key_slot: KEY_SLOT_VERCEL,
    },
    RouterRoute {
        slug: "google",
        label: "Google direkte",
        endpoint: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
        model: "gemini-3.1-flash-lite",
        decoration: Decoration::None,
        hedge: false,
        key_slot: KEY_SLOT_GOOGLE,
    },
    RouterRoute {
        slug: "openrouter",
        label: "OpenRouter",
        endpoint: "https://openrouter.ai/api/v1/chat/completions",
        model: "google/gemini-3.1-flash-lite",
        decoration: Decoration::None,
        hedge: false,
        key_slot: KEY_SLOT_OPENROUTER,
    },
    // Den ENESTE rute der ikke kræver en konto ud over OpenAI: STT ligger
    // allerede dér, så med denne kan hele stemme-vejen køre på én nøgle.
    // Målt mod voice-eval 2026-08-02: Gate 1 41/41 og T6 7/7, altså samme
    // præcision som gemini-ruterne — men ~2x latens (p50 1.3 s mod 0.7 s).
    // Det er afvejningen brugeren vælger, ikke en ringere rute.
    RouterRoute {
        slug: "openai",
        label: "OpenAI",
        endpoint: "https://api.openai.com/v1/chat/completions",
        model: "gpt-5.6-luna",
        decoration: Decoration::ReasoningOff,
        hedge: false,
        key_slot: KEY_SLOT_OPENAI,
    },
];

/// Case-sensitivt opslag. Den TOLERANTE laese-side bor i
/// `workspace::normalize_settings`; her er et ukendt slug en fejl.
pub fn stt_route(slug: &str) -> Option<&'static SttRoute> {
    STT_ROUTES.iter().find(|route| route.slug == slug)
}

pub fn router_route(slug: &str) -> Option<&'static RouterRoute> {
    ROUTER_ROUTES.iter().find(|route| route.slug == slug)
}

/// Origins der skal varmes op ved PTT-tryk for de valgte ruter. Returnerer
/// hver origin hoejst en gang, ogsaa naar begge roller peger samme sted.
pub fn warm_origins(stt: &SttRoute, router: &RouterRoute) -> Vec<String> {
    let mut origins: Vec<String> = Vec::with_capacity(2);
    for endpoint in [stt.endpoint, router.endpoint] {
        let https = endpoint.replacen("wss://", "https://", 1);
        let Some(rest) = https.strip_prefix("https://") else {
            continue;
        };
        let host = rest.split('/').next().unwrap_or_default();
        if host.is_empty() {
            continue;
        }
        let origin = format!("https://{host}");
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    origins
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Listen er UDTOEMMENDE og ikke en laengde-check, fordi kravet fra
    /// 2026-07-29 staar ved magt: en ny rute skal have et SVAR paa partials og
    /// domaene-prompt, foer den skrives ind. Falder testen, er det fordi nogen
    /// tilfoejede en rute uden at tage den stilling.
    ///
    /// OpenRouter-STT er stadig vaek, og det skal den blive — samme model, men
    /// uden loebende tekst og uden domaene-ordlisten.
    #[test]
    fn stt_routes_cover_the_supported_slugs() {
        let slugs: Vec<&str> = STT_ROUTES.iter().map(|r| r.slug).collect();
        assert_eq!(slugs, vec!["openai"]);
        assert!(stt_route("openrouter").is_none());
        // De to 4o-modeller er VAEK, ikke omdoebt. Kommer en af slug'erne
        // tilbage, skal det vaere en bevidst handling og ikke en genopstaaen.
        assert!(stt_route("openai-mini").is_none());
    }

    /// Afloeser `mini_differs_from_the_default_route_only_by_model`, som forsvandt
    /// med mini-ruten 2026-08-05.
    ///
    /// Den test var den ENESTE der haandhaevede at rutens kapabilitets-flag var
    /// taget stilling til — `stt_routes_cover_the_supported_slugs` ovenfor
    /// asserterer trods sin doc-kommentar kun paa slug-listen. Uden en
    /// afloeser ville nedlaeggelsen af den anden rute altsaa have SAENKET
    /// daekningen, hvilket er den slags tab man ikke opdager fordi ingen test
    /// bliver roed af det.
    ///
    /// Assertionerne er positive og ikke sammenlignende: med kun én rute er der
    /// intet at spejle imod, saa hvert felt maa staa ved navn.
    #[test]
    fn the_only_stt_route_states_its_capabilities() {
        let route = stt_route(DEFAULT_STT_SLUG).expect("default stt route");
        assert_eq!(route.model, "gpt-transcribe");
        // MAALT 2026-08-05 paa den raa realtime-log, ikke raesonneret frem.
        assert!(route.supports_partials);
        assert!(route.supports_domain_prompt);
        // gpt-transcribe tager `languages` (array). Sendes `language` i stedet,
        // svarer API'et 200 og ignorerer hintet i stilhed — se `LanguageField`.
        assert_eq!(route.language_field, LanguageField::Plural);
        // Maalt: de to 4o-modeller AFVISER feltet med 400, saa flaget er en
        // gate mod at braekke hver ytring — ikke mod at miste et hint.
        assert!(route.supports_keywords);
    }

    /// STT-ruten er OpenAI og deler noeglen med OpenAI-routeren — ellers kunne
    /// man vaelge en rute, hvis noegle der ikke findes et felt til i UI'et.
    ///
    /// Bemaerk hvad testen IKKE beviser: `secrets::stt_api_key_from` slaar
    /// HAARDKODET op i KEY_SLOT_OPENAI og laeser aldrig `route.key_slot`.
    /// Feltet er i dag rent beskrivende paa STT-siden (eneste laeser er
    /// noegle-badgen i Settings.tsx). Faar en STT-rute et andet slot, skal
    /// `stt_api_key_from` roeres FOERST — ellers henter stemmen den forkerte
    /// noegle, og hverken denne test eller nogen anden falder over det.
    #[test]
    fn stt_routes_use_the_openai_key_slot() {
        for route in STT_ROUTES {
            assert_eq!(route.key_slot, KEY_SLOT_OPENAI, "stt-rute {}", route.slug);
        }
    }

    #[test]
    fn router_routes_cover_the_supported_slugs() {
        let slugs: Vec<&str> = ROUTER_ROUTES.iter().map(|r| r.slug).collect();
        assert_eq!(slugs, vec!["vercel", "google", "openrouter", "openai"]);
    }

    /// OpenAI-ruten findes for at gøre appen brugbar med ÉN nøgle: STT-ruten
    /// bruger allerede `provider_key_openai`, så den der vælger denne router
    /// slipper for en Google-/Vercel-/OpenRouter-konto. Deler de to roller
    /// nøgle-slot, er hele stemme-vejen dækket af det ene slot.
    #[test]
    fn openai_router_shares_the_stt_key_slot() {
        let router = router_route("openai").expect("openai router route");
        let stt = stt_route(DEFAULT_STT_SLUG).expect("default stt route");
        assert_eq!(router.key_slot, stt.key_slot);
        assert_eq!(router.key_slot, KEY_SLOT_OPENAI);
        assert_eq!(router.model, "gpt-5.6-luna");
    }

    /// Uden `reasoning_effort: "none"` afviser GPT-5.6 function tools helt paa
    /// chat/completions — og routeren ER et tvunget tool-kald. Forsvinder
    /// dekorationen, er ruten doed for ENHVER ytring, ikke bare ringere.
    #[test]
    fn gpt5_route_must_carry_reasoning_off() {
        let router = router_route("openai").expect("openai router route");
        assert_eq!(router.decoration, Decoration::ReasoningOff);
    }

    #[test]
    fn defaults_resolve_to_todays_behaviour() {
        let stt = stt_route(DEFAULT_STT_SLUG).expect("default stt route");
        assert_eq!(stt.model, "gpt-transcribe");
        assert_eq!(
            stt.endpoint,
            "wss://api.openai.com/v1/realtime?intent=transcription"
        );
        assert!(stt.supports_partials);
        assert!(stt.supports_domain_prompt);
        assert_eq!(stt.language_field, LanguageField::Plural);
        assert_eq!(stt.key_slot, "provider_key_openai");

        let router = router_route(DEFAULT_ROUTER_SLUG).expect("default router route");
        assert_eq!(router.model, "google/gemini-3.1-flash-lite");
        assert_eq!(
            router.endpoint,
            "https://ai-gateway.vercel.sh/v1/chat/completions"
        );
        assert_eq!(router.decoration, Decoration::VercelGateway);
        assert!(router.hedge);
        assert_eq!(router.key_slot, "provider_key_vercel");
    }

    #[test]
    fn hedge_is_vercel_only() {
        for route in ROUTER_ROUTES {
            assert_eq!(
                route.hedge,
                route.slug == "vercel",
                "hedge skal kun vaere taendt paa vercel — {} havde {}",
                route.slug,
                route.hedge
            );
        }
    }

    /// Dekorationen er en egenskab ved ruten, og hver variant hoerer til
    /// praecis én rute. Testen er udtoemmende, saa en ny rute ikke kan snige
    /// sig ind med en dekoration ingen har taget stilling til.
    #[test]
    fn each_route_carries_its_own_decoration() {
        for route in ROUTER_ROUTES {
            let expected = match route.slug {
                "vercel" => Decoration::VercelGateway,
                "openai" => Decoration::ReasoningOff,
                _ => Decoration::None,
            };
            assert_eq!(route.decoration, expected, "rute {}", route.slug);
        }
    }

    #[test]
    fn unknown_slugs_resolve_to_none() {
        assert!(stt_route("deepgram").is_none());
        assert!(stt_route("").is_none());
        assert!(stt_route("OpenAI").is_none(), "opslag er case-sensitivt");
        assert!(router_route("anthropic").is_none());
    }

    #[test]
    fn every_route_uses_https_or_wss() {
        for route in STT_ROUTES {
            assert!(
                route.endpoint.starts_with("https://") || route.endpoint.starts_with("wss://"),
                "stt-rute {} har utrygt skema: {}",
                route.slug,
                route.endpoint
            );
        }
        for route in ROUTER_ROUTES {
            assert!(
                route.endpoint.starts_with("https://"),
                "router-rute {} har utrygt skema: {}",
                route.slug,
                route.endpoint
            );
        }
    }

    /// Reglen er "én nøgleslot pr. UDBYDER, uanset rolle". Den blev tidligere
    /// bevist på OpenRouter, som var både STT og router — men STT-rollen er
    /// slettet (2026-07-29), og i dag overlapper de to tabeller ikke.
    ///
    /// Testen er derfor tom i dag, og det er med vilje: skrives en udbyder ind
    /// i BEGGE tabeller igen, skal den dele slot, og så fanger den her det.
    /// Alternativet — at slette reglen sammen med sit ene eksempel — ville
    /// efterlade invarianten uden vagt netop når den bliver relevant igen.
    #[test]
    fn key_slots_follow_the_provider_not_the_role() {
        for stt in STT_ROUTES {
            let Some(router) = router_route(stt.slug) else {
                continue;
            };
            assert_eq!(
                stt.key_slot, router.key_slot,
                "udbyderen {} optraeder i begge roller og skal dele noeglen",
                stt.slug
            );
        }
    }

    #[test]
    fn warm_origins_dedupes_and_upgrades_wss() {
        let stt = stt_route("openai").unwrap();
        let router = router_route("vercel").unwrap();
        assert_eq!(
            warm_origins(stt, router),
            vec![
                "https://api.openai.com".to_string(),
                "https://ai-gateway.vercel.sh".to_string(),
            ]
        );

        // Dedupe kan ikke laengere fremprovokeres af to RIGTIGE ruter: efter
        // sletningen af OpenRouter-STT findes der ingen udbyder i begge roller.
        // Egenskaben skal alligevel holde — en fremtidig udbyder kan sagtens
        // dele origin paa tvaers af rollerne — saa parret konstrueres her.
        let samme_vaert = RouterRoute {
            slug: "syntetisk",
            label: "Syntetisk",
            endpoint: "https://api.openai.com/v1/chat/completions",
            model: "uden betydning",
            decoration: Decoration::None,
            hedge: false,
            key_slot: KEY_SLOT_OPENAI,
        };
        assert_eq!(
            warm_origins(stt_route("openai").unwrap(), &samme_vaert),
            vec!["https://api.openai.com".to_string()],
            "samme origin i begge roller skal kun varmes én gang"
        );
    }
}
