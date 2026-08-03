//! "Agenten venter på dig" pr. terminal-kort.
//!
//! Kan IKKE udledes af submit-readiness (spec §0): den står READY gennem hele en
//! manuelt tastet arbejdsgang, og permission-prompts genkendes ikke som prompts.
//! Signalet her er selvstændigt og fodres fra PTY-readeren, samme sted som
//! prompt_readiness::observe.

use std::sync::{Mutex, OnceLock};

use super::AttentionKind;

/// Hvor længe der skal være stille før vi kalder agenten færdig.
pub const SILENCE_MS: u64 = 2_000;

/// Den ENESTE `source`-værdi der tæller som "ejeren svarede". Se `on_input`.
pub const HUMAN: &str = "human";

/// MONOTONT procesur i millisekunder, målt fra første kald (kendelse
/// CORRECTIONS.md C-T7). Maskinen tager tiden som parameter (så tests kan styre
/// den); det er dette kald, de levende fodringsveje bruger. `dispatch::now_ms`
/// duer ikke: den går gennem trådkanalens seam og svarer 0, når den ikke er sat.
///
/// Et vægur (`SystemTime::now`) duer heller ikke. Stilheds-reglen er
/// `now_ms.saturating_sub(last_output_ms) >= SILENCE_MS`, og begge tal ville
/// komme derfra: springer uret BAGUD (W32Time-step, resume fra dvale,
/// RTC-korrektion), ligger `last_output_ms` i fremtiden, `saturating_sub` giver
/// 0, og et færdigt kort kan ikke tænde prikken i hele springets længde.
/// Springer det FREMAD, lyser hvert `dirty` kort øjeblikkeligt — præcis den
/// adfærd rev 1.0 blev forkastet for. En `Instant` kan pr. konstruktion ikke
/// springe.
pub fn now_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// Al foranderlig tilstand under ÉN lås. Separate atomics ville lade
/// PTY-readeren og badge-tick'et se et halvt opdateret billede
/// (fx `dirty` sat, men `last_output_ms` fra forrige chunk).
struct Inner {
    /// Har kortet produceret output siden workspacet sidst var synligt?
    dirty: bool,
    /// Har vi set et eksplicit ventemønster (permission/confirm)?
    prompted: bool,
    workspace_visible: bool,
    last_output_ms: u64,
    /// Sidste bytes fra forrige chunk, så et mønster på chunk-grænsen findes.
    /// Bundet til `max_pattern_len - 1` — aldrig af outputmængden.
    tail: Vec<u8>,
    /// Genbrugt buffer til grænse-vinduet. Bor her frem for som en lokal
    /// `Vec`, så `on_output` ikke allokerer i steady state — den kaldes for
    /// HVER chunk på reader-tråden, som per `pty.rs`' FUND 2 altid skal dræne.
    scratch: Vec<u8>,
}

pub struct CardAttention {
    patterns: &'static [&'static [u8]],
    max_pattern_len: usize,
    inner: Mutex<Inner>,
}

impl CardAttention {
    pub fn new(patterns: &'static [&'static [u8]]) -> Self {
        Self {
            patterns,
            max_pattern_len: patterns.iter().map(|p| p.len()).max().unwrap_or(0),
            inner: Mutex::new(Inner {
                dirty: false,
                prompted: false,
                workspace_visible: true,
                last_output_ms: 0,
                tail: Vec::new(),
                scratch: Vec::new(),
            }),
        }
    }

    #[cfg(test)]
    fn tail_len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tail
            .len()
    }

    pub fn on_output(&self, now_ms: u64, bytes: &[u8]) {
        let mut i = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        i.last_output_ms = now_ms;

        // To søgninger i stedet for én over `hale ++ chunk`, og de dækker
        // tilsammen præcis det samme:
        //
        //   - et mønster HELT inde i chunket findes af `contains(bytes, ..)`;
        //   - et mønster der rører halen — enten fordi det krydser grænsen
        //     eller fordi det ligger helt i halen — kan højst nå
        //     `max_pattern_len - 1` bytes ind i chunket, så det findes i
        //     grænse-vinduet `hale ++ chunk[..overlap]`.
        //
        // Den gamle form byggede `hale ++ HELE chunket` op i en frisk `Vec`
        // og skrev derefter halen tilbage med endnu en: to allokeringer plus
        // en memcpy af hele chunket, per chunk, på drain-tråden.
        let overlap = self.max_pattern_len.saturating_sub(1);
        let i = &mut *i;
        i.scratch.clear();
        if !i.tail.is_empty() {
            i.scratch.extend_from_slice(&i.tail);
            i.scratch
                .extend_from_slice(&bytes[..overlap.min(bytes.len())]);
        }
        let matchede = self
            .patterns
            .iter()
            .any(|p| contains(bytes, p) || contains(&i.scratch, p));

        // Behold kun så meget hale som det længste mønster kan spænde over.
        // Opdateres på plads, så kapaciteten genbruges.
        let behold = overlap.min(i.tail.len() + bytes.len());
        if bytes.len() >= behold {
            i.tail.clear();
            i.tail.extend_from_slice(&bytes[bytes.len() - behold..]);
        } else {
            let fra_hale = behold - bytes.len();
            i.tail.drain(..i.tail.len() - fra_hale);
            i.tail.extend_from_slice(bytes);
        }

        if i.workspace_visible {
            // Du kigger på det: nyt output betyder at agenten arbejder igen, så
            // der er intet uset at gøre opmærksom på.
            i.dirty = false;
            // MEN mønster-viden kasseres IKKE. `prompted` er kortets FAKTISKE
            // situation lige nu ("står med en ubesvaret prompt"), ikke en
            // notifikations-tilstand — og skifter ejeren workspace i næste
            // øjeblik, er det præcis den viden `on_conceal` skal bære med over.
            // Nyt output der IKKE matcher betyder omvendt at agenten kørte
            // videre, så flaget falder igen.
            i.prompted = matchede;
            return;
        }
        i.dirty = true;
        if matchede {
            i.prompted = true;
        }
    }

    /// Ejeren svarede. To gates, og begge er nødvendige:
    ///
    /// 1. **Kun `source == "human"`.** `write_pty` er en delt kanal: xterm's
    ///    protokol-auto-svar (fokus `\x1b[I`/`\x1b[O` på mode 1004 — CC enabler
    ///    den; DA/CPR; OSC 10/11/12-tema-queries) og agent-drevne skrivninger går
    ///    samme vej, klassificeret `"terminal"` hhv. `"persona"` (fix F1). Uden
    ///    denne gate ville selve dét at kigge på workspacet slukke prikken:
    ///    webviewet tager fokus, xterm svarer `\x1b[I`, og ejeren har ikke svaret
    ///    på noget.
    /// 2. **Kun når workspacet er synligt** — ellers kunne en skrivning rydde en
    ///    prik ejeren aldrig så.
    pub fn on_input(&self, source: &str) {
        if source != HUMAN {
            return;
        }
        let mut i = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if i.workspace_visible {
            i.dirty = false;
            i.prompted = false;
        }
    }

    /// Ejeren kiggede væk. Herfra tæller output som noget ingen ser.
    pub fn on_conceal(&self) {
        let mut i = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        i.workspace_visible = false;
        // Ejer-beslutning 9's kerne: en agent der ALLEREDE ventede da ejeren
        // kiggede væk, skal kunne tænde prikken. Kortet udsender pr. definition
        // intet mere mens det står på en permission-prompt, så var `prompted`
        // ryddet her, ville intet nogensinde tænde den igen.
        //
        // `dirty` afledes af `prompted` frem for at blive sat ubetinget: et kort
        // der bare stod i tomgang mens ejeren kiggede på det, har ejeren selv
        // set stå stille, og stilheds-reglen må ikke gøre det til en prik i det
        // øjeblik der skiftes workspace.
        i.dirty = i.prompted;
        i.tail.clear();
    }

    /// Bemærk: rydder IKKE prikken. Venter agenten stadig, bliver den stående —
    /// en prik der forsvinder ved et blik ville lyve.
    pub fn on_reveal(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .workspace_visible = true;
    }

    pub fn poll_kind(&self, now_ms: u64) -> AttentionKind {
        let i = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if !i.dirty {
            return AttentionKind::None;
        }
        if i.prompted {
            return AttentionKind::NeedsYou;
        }
        if now_ms.saturating_sub(i.last_output_ms) >= SILENCE_MS {
            AttentionKind::DoneUnread
        } else {
            AttentionKind::None
        }
    }

    pub fn poll(&self, now_ms: u64) -> bool {
        self.poll_kind(now_ms).is_attention()
    }
}

/// Bringer en NYINSTALLERET maskine i sync med workspacets synlighed.
///
/// # Hvorfor den findes
///
/// `spawn_into` læser synligheden FØR kortlåsen tages (låseorden: StatusWriter
/// må aldrig låses med en `CardRuntime`-lås i hånden) og bruger den først EFTER
/// hele PTY-spawnet, hundreder af ms senere. I mellemtiden kan pollertråden nå
/// hele sin Reveal-gren — og dens `set_attention_visibility(true)` finder ingen
/// maskine på kortet endnu, fordi den først installeres til sidst. Uden denne
/// funktion stod maskinen `workspace_visible = false` mens workspacet var
/// fremme, og efter 2 s stilhed tændte prikken på det workspace ejeren sad og
/// kiggede på. Kanttrigget som `set_attention_visibility` er, rettede det sig
/// først ved næste conceal→reveal.
///
/// # Hvorfor ÉN genlæsning ikke er nok
///
/// Kanten kan falde mellem vores læsning og vores anvendelse — så ville vi
/// overskrive den værdi polleren netop havde sat. Derfor læses der igen efter
/// hver anvendelse, indtil kilden er enig med det vi sidst satte.
///
/// # Hvorfor det terminerer korrekt
///
/// Kilden (`workspaces::attention_visibility_now`) stemples af
/// `set_attention_visibility` FØR den itererer registryet. Maskinen er
/// installeret under kortlåsen, altså før første læsning her. Enhver kant der
/// falder efter vores sidste læsning når derfor maskinen gennem pollerens egen
/// iteration; enhver kant før vores sidste læsning ser vi selv. Loftet er et
/// rent værn mod en patologisk kilde — polleren kanter højst hver 200 ms.
///
/// `None` = kilden har aldrig talt (CLI, tests, og vinduet før første reveal).
/// Da rører vi ikke maskinen: fødselsværdien fra `spawn_into` står.
pub fn sync_initial_visibility(
    machine: &CardAttention,
    mut nuvaerende: impl FnMut() -> Option<bool>,
    foedt_synlig: bool,
) {
    let mut anvendt = foedt_synlig;
    for _ in 0..8 {
        let Some(synlig) = nuvaerende() else { return };
        if synlig == anvendt {
            return;
        }
        // Kun ved uenighed: `on_conceal` afleder `dirty` af `prompted`, så en
        // overflødig conceal ville slukke en prik tændt af stilhed.
        if synlig {
            machine.on_reveal();
        } else {
            machine.on_conceal();
        }
        anvendt = synlig;
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOENSTRE: &[&[u8]] = &[b"Do you want to proceed?"];

    fn skjult_kort() -> CardAttention {
        let a = CardAttention::new(MOENSTRE);
        a.on_conceal();
        a
    }

    #[test]
    fn kind_daekker_alle_fire_betingelser() {
        let ren = skjult_kort();
        assert_eq!(ren.poll_kind(99_000), AttentionKind::None, "!dirty");

        let arbejder = skjult_kort();
        arbejder.on_output(1_000, b"arbejder\n");
        assert_eq!(
            arbejder.poll_kind(1_000 + SILENCE_MS - 1),
            AttentionKind::None,
            "dirty uden prompt eller stilhed"
        );
        assert_eq!(
            arbejder.poll_kind(1_000 + SILENCE_MS),
            AttentionKind::DoneUnread,
            "dirty og stille"
        );

        let prompt = skjult_kort();
        prompt.on_output(1_000, b"Do you want to proceed?");
        assert_eq!(
            prompt.poll_kind(1_001),
            AttentionKind::NeedsYou,
            "prompt vinder uden at vente paa stilhed"
        );
    }

    /// `on_output` søger i to vinduer (chunket selv, og hale ++ chunkets
    /// første `max_pattern_len-1` bytes) frem for i én sammensat buffer.
    ///
    /// Krydsnings-tilfældet er dækket ovenfor; DETTE er det andet tilfælde der
    /// kan afsløre en forskel: et mønster som ligger HELT inde i den bevarede
    /// hale. Det kan ske, fordi halen er lige så lang som det LÆNGSTE mønster
    /// minus én — og et kortere mønster er der derfor plads til. Testen kigger
    /// på den SYNLIGE gren, hvor `prompted = matchede` sættes forfra ved hvert
    /// chunk: en implementation der kun søgte i chunket ville tabe flaget her,
    /// og det ville først vise sig som en manglende prik efter et skift væk.
    #[test]
    fn moenster_der_kun_ligger_i_halen_taeller_stadig() {
        const TO: &[&[u8]] = &[b"Do you want to proceed?", b"Do you want to make this edit"];
        let a = CardAttention::new(TO);
        a.on_reveal();
        a.on_output(1_000, b"Do you want to proceed?");
        // Chunk uden eget match; mønsteret findes nu kun i halen.
        a.on_output(1_001, b"x");
        a.on_conceal();
        assert_eq!(
            a.poll_kind(1_002),
            AttentionKind::NeedsYou,
            "prompt-viden fra halen skal baeres med over conceal-kanten"
        );
    }

    #[test]
    fn output_alene_taender_ikke_prikken() {
        let a = skjult_kort();
        a.on_output(1_000, b"tool call...\n");
        assert!(
            !a.poll(1_500),
            "agenten arbejder stadig — prikken må ikke lyse"
        );
    }

    #[test]
    fn stilhed_over_taersklen_taender_prikken() {
        let a = skjult_kort();
        a.on_output(1_000, b"faerdig\n");
        assert!(
            a.poll(1_000 + SILENCE_MS),
            "efter 2,0 s stilhed venter agenten"
        );
    }

    #[test]
    fn attention_moenster_taender_straks() {
        let a = skjult_kort();
        a.on_output(1_000, b"Do you want to proceed? (y/n)");
        assert!(
            a.poll(1_050),
            "en permission-prompt skal lyse med det samme, ikke efter stilhed"
        );
    }

    #[test]
    fn intet_output_siden_conceal_giver_ingen_prik() {
        let a = skjult_kort();
        assert!(
            !a.poll(999_999),
            "et kort der intet har lavet, venter ikke på dig"
        );
    }

    #[test]
    fn synligt_kort_taender_aldrig() {
        let a = CardAttention::new(MOENSTRE);
        a.on_reveal();
        a.on_output(1_000, b"Do you want to proceed?");
        assert!(!a.poll(9_000), "du kigger jo på det");
    }

    #[test]
    fn input_rydder_prikken_men_kun_naar_workspacet_er_synligt() {
        let a = skjult_kort();
        a.on_output(1_000, b"Do you want to proceed?");
        assert!(a.poll(1_050));

        a.on_input("human");
        assert!(
            a.poll(1_100),
            "input mens workspacet er skjult må ikke rydde — du har ikke set det"
        );

        a.on_reveal();
        assert!(
            a.poll(1_150),
            "et blik alene rydder ikke: agenten venter stadig"
        );
        a.on_input("human");
        assert!(!a.poll(1_200), "nu har du svaret");
    }

    #[test]
    fn nyt_output_efter_reveal_rydder_fordi_agenten_arbejder_igen() {
        let a = skjult_kort();
        a.on_output(1_000, b"faerdig\n");
        assert!(a.poll(1_000 + SILENCE_MS));

        a.on_reveal();
        a.on_output(5_000, b"arbejder videre\n");
        assert!(!a.poll(5_100));
    }

    #[test]
    fn moenster_delt_over_to_chunks_findes_stadig() {
        // PTY'en leverer vilkårlige chunks. Uden en hale ville et mønster der
        // falder på grænsen forsvinde — og det er præcis permission-prompten,
        // som er hele pointen med prikken.
        let a = skjult_kort();
        a.on_output(1_000, b"...Do you want to ");
        a.on_output(1_010, b"proceed? (y/n)");
        assert!(
            a.poll(1_020),
            "mønsteret skal findes på tværs af chunk-grænsen"
        );
    }

    #[test]
    fn skjult_prompt_overlever_et_repaint_chunk_uden_moensteret() {
        let a = skjult_kort();
        a.on_output(1_000, b"Do you want to proceed? (y/n)");
        assert_eq!(a.poll_kind(1_001), AttentionKind::NeedsYou);

        // Alt-screen-TUI'er kan sende cursor-/box-repaints mens de venter.
        // Uden en live-capture der beviser at hvert repaint gentager prompten,
        // er den sikre adfærd at bevare promptflaget.
        a.on_output(1_010, b"\x1b[2A\x1b[2K\r");
        assert_eq!(
            a.poll_kind(1_011),
            AttentionKind::NeedsYou,
            "et ikke-matchende repaint maa ikke nedgradere en skjult permission-prompt"
        );
    }

    #[test]
    fn en_agent_der_allerede_ventede_da_du_kiggede_vaek_taender_prikken() {
        // Ejer-beslutning 9's kerne-use-case. Kortet staar med en ubesvaret
        // permission-prompt MENS workspacet er fremme; ejeren skifter væk.
        // Agenten er blokeret og udsender INTET mere — er prompt-viden ikke
        // baaret over conceal-kanten, kan prikken aldrig taende.
        let a = CardAttention::new(MOENSTRE);
        a.on_reveal();
        a.on_output(1_000, b"Do you want to proceed? (y/n)");
        assert!(
            !a.poll(1_100),
            "du kigger paa det — prikken maa ikke lyse endnu"
        );

        a.on_conceal();
        assert!(
            a.poll(1_200),
            "agenten ventede allerede da du kiggede vaek — prikken skal taende uden nyt output"
        );
        // Spec §2.4: conceal-kantens `dirty = prompted` baerer prompt-viden med
        // over, saa den skal eskalere. Lander den paa done_unread, siger rail'en
        // "faerdig" om en blokeret agent.
        assert_eq!(
            a.poll_kind(1_200),
            AttentionKind::NeedsYou,
            "conceal-kanten maa ikke nedgradere en ubesvaret prompt til done_unread"
        );
    }

    #[test]
    fn et_kort_i_tomgang_du_selv_saa_taender_ikke_ved_conceal() {
        // Modstykket til testen ovenfor, og grunden til at conceal baerer
        // `prompted` og ikke bare saetter `dirty = true`: et kort der blev
        // faerdigt for laenge siden mens du kiggede paa det, har du allerede
        // set. Det maa ikke lyse, bare fordi du skifter workspace.
        let a = CardAttention::new(MOENSTRE);
        a.on_reveal();
        a.on_output(1_000, b"faerdig\n");
        a.on_conceal();
        assert!(
            !a.poll(1_000 + SILENCE_MS * 10),
            "du har selv set det staa stille"
        );
    }

    #[test]
    fn et_svar_agenten_selv_afgiver_rydder_ikke_prikken() {
        // xterm's protokol-auto-svar (fokus \x1b[I/\x1b[O paa mode 1004, DA/CPR,
        // OSC 10/11/12) gaar gennem SAMME write_pty som menneskets tastetryk,
        // men er klassificeret `source:"terminal"` (fix F1). Sluger maskinen
        // dem som "brugeren svarede", slukkes prikken af et fokusskift.
        let a = skjult_kort();
        a.on_output(1_000, b"Do you want to proceed?");
        assert!(a.poll(1_050));

        a.on_reveal(); // ejeren kigger paa workspacet; webviewet tager fokus
        a.on_input("terminal"); // \x1b[I fra xterm — ikke et svar
        assert!(a.poll(1_100), "et terminal-autosvar er ikke ejerens svar");
        a.on_input("persona"); // agent-drevet skrivning — heller ikke et svar
        assert!(
            a.poll(1_150),
            "agentens egen skrivning er ikke ejerens svar"
        );

        a.on_input("human");
        assert!(!a.poll(1_200), "nu har ejeren faktisk svaret");
    }

    #[test]
    fn now_ms_er_et_monotont_procesur_ikke_et_vaegur() {
        // Kendelse CORRECTIONS.md C-T7: uret skal vaere monotont. Et vaegur
        // (SystemTime) kan springe bagud (W32Time-step, dvale-resume) og laase
        // stilheds-reglen i op til springets laengde — eller springe fremad og
        // taende hvert dirty kort oejeblikkeligt. Epoke-millisekunder er
        // ~1,7e12; et procesur startet ved foerste kald kan ikke komme i
        // naerheden af den graense i en testkoersel.
        let foer = now_ms();
        assert!(
            foer < 1_000_000_000,
            "now_ms lignede et vaegur ({foer}) — den skal maale fra procesens start"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(now_ms() >= foer, "uret maa aldrig gaa baglaens");
    }

    #[test]
    fn en_reveal_under_spawnet_maa_ikke_efterlade_maskinen_skjult() {
        // TOCTOU'en i `spawn_into`: synligheden blev laest FOER kortlaasen og
        // anvendt EFTER hele PTY-spawnet. Pollertraaden naaede sin Reveal-gren
        // imens — men dens `set_attention_visibility(true)` fandt ingen maskine
        // paa kortet endnu. Kortet blev derfor foedt skjult, og efter 2 s
        // stilhed taendte prikken paa det workspace ejeren sad og kiggede paa.
        let a = CardAttention::new(MOENSTRE);
        a.on_conceal(); // `spawn_into`s foedsels-conceal (workspacet VAR skjult)

        // Da spawnet er faerdigt, er workspacet fremme.
        sync_initial_visibility(&a, || Some(true), false);

        a.on_output(1_000, b"faerdig\n");
        assert!(
            !a.poll(1_000 + SILENCE_MS * 5),
            "workspacet er fremme — prikken maa ikke taende paa det ejeren kigger paa"
        );
    }

    #[test]
    fn synkroniseringen_genlaeser_indtil_kilden_er_enig() {
        // Kanten kan ogsaa falde MELLEM vores laesning og vores anvendelse: da
        // har polleren allerede sat maskinen, og vi overskriver den. Én
        // genlaesning-og-anvendelse er derfor ikke nok — der skal laeses igen
        // efter hver anvendelse.
        let kald = std::cell::Cell::new(0u32);
        let kilde = || {
            let n = kald.get();
            kald.set(n + 1);
            // Foerste laesning ser den gamle vaerdi; kanten falder lige efter.
            Some(n > 0)
        };

        let a = CardAttention::new(MOENSTRE);
        sync_initial_visibility(&a, kilde, true);
        assert!(
            kald.get() >= 3,
            "kilden skal laeses igen efter hver anvendelse"
        );

        a.on_output(1_000, b"faerdig\n");
        assert!(
            !a.poll(1_000 + SILENCE_MS * 5),
            "sidste ord er kildens: maskinen skal ende synlig"
        );
    }

    #[test]
    fn en_kilde_der_aldrig_har_talt_roerer_ikke_maskinen() {
        // CLI og tests har ingen pollertraad, og vinduet har ingen kant foer
        // foerste reveal. Da skal foedselsvaerdien fra `spawn_into` staa.
        let a = CardAttention::new(MOENSTRE);
        a.on_conceal();
        sync_initial_visibility(&a, || None, false);
        a.on_output(1_000, b"faerdig\n");
        assert!(
            a.poll(1_000 + SILENCE_MS),
            "uden en kilde skal kortet blive staaende skjult og kunne taende prikken"
        );
    }

    #[test]
    fn en_kilde_der_er_enig_roerer_ikke_maskinen() {
        // Idempotens: `on_conceal` saetter `dirty = prompted`. Blev den kaldt
        // igen oven i en allerede skjult maskine, ville en prik taendt af
        // stilhed (prompted == false) blive slukket.
        let a = CardAttention::new(MOENSTRE);
        a.on_conceal();
        a.on_output(1_000, b"faerdig\n");
        assert!(a.poll(1_000 + SILENCE_MS), "prikken er taendt af stilhed");

        sync_initial_visibility(&a, || Some(false), false);
        assert!(
            a.poll(1_000 + SILENCE_MS),
            "en enig kilde maa ikke give en overfloedig conceal, der ville rydde prikken"
        );
    }

    #[test]
    fn halen_vokser_ikke_ubegraenset() {
        let a = CardAttention::new(MOENSTRE);
        a.on_conceal();
        for i in 0..1_000 {
            a.on_output(1_000 + i, b"en hel masse output der aldrig matcher noget\n");
        }
        assert!(
            a.tail_len() < 64,
            "halen skal være bundet af det længste mønster, ikke af outputmængden"
        );
    }
}
