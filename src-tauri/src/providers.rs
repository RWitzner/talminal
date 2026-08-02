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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SttRoute {
    pub slug: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub model: &'static str,
    /// Falsk => HUD'en faar ingen loebende tekst, og turen skal ryddes
    /// eksplicit ved start (spec §4).
    ///
    /// Er sand for den eneste rute i dag. Feltet bliver staaende, fordi det
    /// beskriver RUTEN og styrer rigtig adfaerd i pipelinen — modsat
    /// `transport`, hvis eneste opgave var at vaelge en kodesti der nu er
    /// slettet.
    pub supports_partials: bool,
    /// Falsk => STT_DOMAIN_PROMPT ignoreres af udbyderen; dansk-genkendelsen
    /// paa kort-numre bliver maalbart ringere. Skal maerkes i UI'et.
    pub supports_domain_prompt: bool,
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

/// ÉN rute. Tabellen bevares som tabel — ikke fordi der er noget at vælge
/// imellem i dag, men fordi den er stedet en ny udbyder skal skrives ind, og
/// fordi wiren (`voice_routes.stt`) allerede taler dens sprog.
pub const STT_ROUTES: &[SttRoute] = &[SttRoute {
    slug: "openai",
    label: "OpenAI",
    endpoint: "wss://api.openai.com/v1/realtime?intent=transcription",
    model: "gpt-4o-transcribe",
    supports_partials: true,
    supports_domain_prompt: true,
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

    /// OpenRouter-STT blev slettet 2026-07-29 (ejer-beslutning). Testen holder
    /// fast i, at der er PRÆCIS én rute — kommer der en til, skal nogen tage
    /// stilling til partials og domæne-prompt for den rute, og det er her det
    /// bliver opdaget.
    #[test]
    fn stt_has_exactly_one_route() {
        let slugs: Vec<&str> = STT_ROUTES.iter().map(|r| r.slug).collect();
        assert_eq!(slugs, vec!["openai"]);
        assert!(stt_route("openrouter").is_none());
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
        assert_eq!(stt.model, "gpt-4o-transcribe");
        assert_eq!(
            stt.endpoint,
            "wss://api.openai.com/v1/realtime?intent=transcription"
        );
        assert!(stt.supports_partials);
        assert!(stt.supports_domain_prompt);
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
