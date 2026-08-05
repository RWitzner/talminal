//! Voice-wake-hotkey: niveau-baseret kant-detektion paa Rust-siden.
//!
//! HVORFOR (2026-07-19, "foerste tryk er doedt"-symptomet): den rene
//! DOM-keydown-vej (ptt.ts::registerWakeKey) afhaenger af at Windows leverer
//! tasteeventet ind i WebView2'ens DOM. Leverancen fejler efter fokus-
//! transitioner (vinduesaktivering hvor webviewet endnu ikke har keyboard-
//! fokus, andre apps' hooks, mistede keyups) — trykket forsvinder lydloest.
//! Redapting-companionen rodaarsagede det identiske symptom (fund 2026-07-03,
//! desktop/src-tauri/src/hotkey/voice_hook.rs) og fixet er portet herfra:
//!
//! POLLEREN er den ENESTE kilde til wake-kanter. Den laeser den fysiske
//! taste-tilstand via GetAsyncKeyState hvert POLL_INTERVAL_MS og udleder
//! kanter af NIVEAU, ikke af leverede events — immun over for alle de maader
//! event-leverance kan fejle paa. Et mistet keyup kan aldrig efterlade en
//! haengende tilstand: naeste poll laeser bare sandheden.
//!
//! Afvigelser fra redapting-forlaegget (bevidste):
//! - INGEN WH_KEYBOARD_LL-hook: canvas' hotkey er vindues-lokal, saa
//!   suppression (Space maa ikke naa terminalen) klares af DOM-handleren i
//!   ptt.ts — den fyrer praecis naar DOM'en kan modtage tasten, hvilket er
//!   praecis naar suppression behoves. Dermed bortfalder ogsaa hele
//!   "aedt trigger-evidens"-maskineriet (async-tabellen er altid sandfaerdig
//!   naar ingen events aedes).
//! - FOKUS-GATE: kanten emitter kun naar canvas-vinduet er forgrundsvinduet.
//!   Sandheden laeses NIVEAU-baseret pr. poll via GetForegroundWindow mod
//!   canvas' HWND — ALDRIG fra WindowEvent::Focused-events (fix 2026-07-20,
//!   "4-5 doede tryk": den foerste port bogfoerte gaten fra fokus-events, og
//!   WebView2-fokusevents kan udeblive/omrokeres ved aktiveringsovergange,
//!   saa gaten stod stale-false mens canvas reelt havde fokus — praecis den
//!   event-leverance-afhaengighed polleren blev bygget til at undslippe).
//!   Hold-tilstanden foelger den fysiske trigger-tast alene, saa et hold der
//!   STARTER ufokuseret aldrig fyrer ved fokus-ankomst, og modifier-vrid
//!   midt i et hold aldrig dobbelt-fyrer.
//! - MODIFIER-SEMANTIK = forlaeggets SUBSET-match (rettet 2026-07-20 —
//!   porten valgte oprindeligt eksakt lighed spejlet af ptt.ts, og det doede
//!   tavst paa fantom-modifier-bits i async-tabellen): kraevede modifiers
//!   skal vaere nede, ekstra modifiers ignoreres. ptt.ts::matchesAccelerator
//!   foelger samme semantik (lagene SKAL matche ens — DOM-suppression og
//!   poller-kanter deler grammatik).

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

/// SKARPERE end redapting-forlaeggets 15 ms (fix 2026-07-20, "samtidigt tryk
/// fejler, langsomt virker altid"): forlaegget er tap-robust via LL-hookens
/// eaten-evidence-kanal (event-drevet trigger-vidne m. TTL) — canvas har
/// bevidst ingen hook, saa polleren skal SELV fange korte tap. Ved 15 ms
/// nominelt var den reelle kadence 15,6-31 ms (Windows' default timer-
/// granularitet), og et hurtigt samtidigt tryk kunne falde HELT mellem to
/// polls (bevist i dogfood: DOM saa komboen, polleren fyrede aldrig).
/// Poller-traaden haever timer-oploesningen med timeBeginPeriod(1), ellers
/// er 5 ms nominelt stadig 15+ reelt.
const POLL_INTERVAL_MS: u64 = 5;
/// Event-navnet foelger wire-konventionen ("pty-output", "card-exit", ...).
pub const WAKE_EVENT: &str = "wake-hotkey";

/// Fysisk trigger-identitet. `code` er det kanoniske v2-token (= DOM'ens
/// `event.code`), `sc` PS/2 Set-1-scancoden og `extended` 0xE0-praefikset.
/// COPY-bar: KeyCombo laeses med `*slot` ud af Mutex'en i poller-loekken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Key {
        code: &'static str,
        sc: u16,
        extended: bool,
    },
    /// Mus: fast VK, ingen layout-involvering.
    Mouse { code: &'static str, vk: u16 },
    /// XInput-gamepad. Hverken VK eller layout er involveret; tilstanden
    /// kommer fra `XInputGetState`, ikke fra `GetAsyncKeyState`.
    ///
    /// `code` SKAL blive staaende som felt, selv om `input` naesten altid er
    /// entydig: `collides` sammenligner trigger-identitet, og de to analoge
    /// triggere baerer samme `PadInput`-diskriminant-data hvis man udelader
    /// koden. Uden feltet ville LT-til-PTT og RT-til-diktering blive afvist
    /// som "samme tast".
    Gamepad {
        code: &'static str,
        input: PadInput,
    },
}

/// Hvor paa pad'en knappen sidder. XInput deler tilstanden i to: 14 bit-flag i
/// `wButtons`, og to ANALOGE triggere i separate `u8`-felter (0-255). En
/// trigger er derfor en taerskel, ikke et bit-tjek.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadInput {
    /// Bit i `XINPUT_GAMEPAD.wButtons`.
    Button(u16),
    /// `bLeftTrigger` / `bRightTrigger`.
    Trigger(PadSide),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadSide {
    Left,
    Right,
}

/// Hvornaar en analog trigger taeller som nede — 0,4 af fuldt udslag.
///
/// Chromium meldte `pressed` allerede ved 0,25, og XInputs egen
/// `XINPUT_GAMEPAD_TRIGGER_THRESHOLD` er 30/255 ≈ 0,12. Begge er for lette:
/// et hvil af pegefingeren ville aabne mikrofonen. Maalt paa Quest-controlleren
/// naaede almindelige tryk 0,94-1,00, saa der er rigelig luft over taersklen.
pub const PAD_TRIGGER_THRESHOLD: u8 = 102;

impl Trigger {
    pub fn code(&self) -> &'static str {
        match self {
            Trigger::Key { code, .. }
            | Trigger::Mouse { code, .. }
            | Trigger::Gamepad { code, .. } => code,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub trigger: Trigger,
}

impl KeyCombo {
    pub fn is_bare(&self) -> bool {
        !self.ctrl && !self.shift && !self.alt
    }
}

/// Parser med SAMME grammatik som ptt.ts::parseAccelerator. Kontrakten er
/// `src/voice/hotkey-grammar.fixtures.json`, som BEGGE lag tester mod — denne
/// fil via `include_str!` nedenfor, frontenden via ptt.test.ts. Flyttes den,
/// skal begge stier med; kun den ene ville lade lagene drive fra hinanden i
/// tavshed, hvilket er praecis dét fixturen findes for at forhindre.
/// Modifiers: ctrl/cmdorctrl/control, shift, alt/option. Praecis een trigger.
/// Bar binding kun for F1-F12 og Mouse1-Mouse5.
pub fn parse_accelerator(accel: &str) -> Result<KeyCombo, String> {
    let parts: Vec<&str> = accel
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err(format!("Uparsebar accelerator: {accel}"));
    }
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut trigger: Option<Trigger> = None;
    for part in parts {
        let token = part.to_lowercase();
        match token.as_str() {
            "cmdorctrl" | "ctrl" | "control" => {
                ctrl = true;
                continue;
            }
            "shift" => {
                shift = true;
                continue;
            }
            "alt" | "option" => {
                alt = true;
                continue;
            }
            _ => {}
        }
        if trigger.is_some() {
            return Err(format!("Uparsebar accelerator: {accel}"));
        }
        trigger = Some(
            token_to_trigger(&token)
                .or_else(|| migrate_v1_token(&token).and_then(|t| token_to_trigger(&t)))
                .ok_or_else(|| format!("Uparsebar accelerator: {accel}"))?,
        );
    }
    let trigger = trigger.ok_or_else(|| format!("Uparsebar accelerator: {accel}"))?;
    let combo = KeyCombo {
        ctrl,
        shift,
        alt,
        trigger,
    };
    if combo.is_bare() && !allows_bare(&trigger) {
        return Err(format!(
            "{accel} kraever mindst een modifier — kun F1-F12, musetaster og gamepad-knapper maa staa alene"
        ));
    }
    Ok(combo)
}

const CODE_TABLE: &[(&str, &str, u16, bool)] = &[
    ("keyq", "KeyQ", 0x10, false),
    ("keyw", "KeyW", 0x11, false),
    ("keye", "KeyE", 0x12, false),
    ("keyr", "KeyR", 0x13, false),
    ("keyt", "KeyT", 0x14, false),
    ("keyy", "KeyY", 0x15, false),
    ("keyu", "KeyU", 0x16, false),
    ("keyi", "KeyI", 0x17, false),
    ("keyo", "KeyO", 0x18, false),
    ("keyp", "KeyP", 0x19, false),
    ("keya", "KeyA", 0x1E, false),
    ("keys", "KeyS", 0x1F, false),
    ("keyd", "KeyD", 0x20, false),
    ("keyf", "KeyF", 0x21, false),
    ("keyg", "KeyG", 0x22, false),
    ("keyh", "KeyH", 0x23, false),
    ("keyj", "KeyJ", 0x24, false),
    ("keyk", "KeyK", 0x25, false),
    ("keyl", "KeyL", 0x26, false),
    ("keyz", "KeyZ", 0x2C, false),
    ("keyx", "KeyX", 0x2D, false),
    ("keyc", "KeyC", 0x2E, false),
    ("keyv", "KeyV", 0x2F, false),
    ("keyb", "KeyB", 0x30, false),
    ("keyn", "KeyN", 0x31, false),
    ("keym", "KeyM", 0x32, false),
    ("digit1", "Digit1", 0x02, false),
    ("digit2", "Digit2", 0x03, false),
    ("digit3", "Digit3", 0x04, false),
    ("digit4", "Digit4", 0x05, false),
    ("digit5", "Digit5", 0x06, false),
    ("digit6", "Digit6", 0x07, false),
    ("digit7", "Digit7", 0x08, false),
    ("digit8", "Digit8", 0x09, false),
    ("digit9", "Digit9", 0x0A, false),
    ("digit0", "Digit0", 0x0B, false),
    ("f1", "F1", 0x3B, false),
    ("f2", "F2", 0x3C, false),
    ("f3", "F3", 0x3D, false),
    ("f4", "F4", 0x3E, false),
    ("f5", "F5", 0x3F, false),
    ("f6", "F6", 0x40, false),
    ("f7", "F7", 0x41, false),
    ("f8", "F8", 0x42, false),
    ("f9", "F9", 0x43, false),
    ("f10", "F10", 0x44, false),
    ("f11", "F11", 0x57, false),
    ("f12", "F12", 0x58, false),
    ("space", "Space", 0x39, false),
    ("escape", "Escape", 0x01, false),
    ("enter", "Enter", 0x1C, false),
    ("tab", "Tab", 0x0F, false),
    ("backspace", "Backspace", 0x0E, false),
    ("arrowup", "ArrowUp", 0x48, true),
    ("arrowleft", "ArrowLeft", 0x4B, true),
    ("arrowright", "ArrowRight", 0x4D, true),
    ("arrowdown", "ArrowDown", 0x50, true),
    ("insert", "Insert", 0x52, true),
    ("delete", "Delete", 0x53, true),
    ("home", "Home", 0x47, true),
    ("end", "End", 0x4F, true),
    ("pageup", "PageUp", 0x49, true),
    ("pagedown", "PageDown", 0x51, true),
    ("minus", "Minus", 0x0C, false),
    ("equal", "Equal", 0x0D, false),
    ("bracketleft", "BracketLeft", 0x1A, false),
    ("bracketright", "BracketRight", 0x1B, false),
    ("backslash", "Backslash", 0x2B, false),
    ("semicolon", "Semicolon", 0x27, false),
    ("quote", "Quote", 0x28, false),
    ("backquote", "Backquote", 0x29, false),
    ("comma", "Comma", 0x33, false),
    ("period", "Period", 0x34, false),
    ("slash", "Slash", 0x35, false),
    ("intlbackslash", "IntlBackslash", 0x56, false),
];

const MOUSE_TABLE: &[(&str, &str, u16)] = &[
    ("mouse1", "Mouse1", 0x01),
    ("mouse2", "Mouse2", 0x02),
    ("mouse3", "Mouse3", 0x04),
    ("mouse4", "Mouse4", 0x05),
    ("mouse5", "Mouse5", 0x06),
];

/// De 16 input XInput faktisk kan rapportere: 14 bit-flag + 2 analoge triggere.
///
/// Bemaerk at Chromiums Gamepad API melder **17** knapper for den samme pad.
/// Den 17. (index 16) er Guide/Home, som ligger paa bit 0x0400 og kun kan
/// laeses via den udokumenterede `XInputGetStateEx` (ordinal 100). Den findes
/// ikke i windows-sys, og en binding til den ville gemme sig lydloest og
/// aldrig fyre — derfor staar den ikke her, og optageren afviser index 16.
///
/// Maskerne er skrevet ud i stedet for at importere `XINPUT_GAMEPAD_*`, saa
/// tabellen ogsaa kompilerer uden for Windows. `masker_matcher_xinput` laaser
/// dem mod de rigtige konstanter.
const GAMEPAD_TABLE: &[(&str, &str, PadInput)] = &[
    ("gamepada", "GamepadA", PadInput::Button(0x1000)),
    ("gamepadb", "GamepadB", PadInput::Button(0x2000)),
    ("gamepadx", "GamepadX", PadInput::Button(0x4000)),
    ("gamepady", "GamepadY", PadInput::Button(0x8000)),
    ("gamepadlb", "GamepadLB", PadInput::Button(0x0100)),
    ("gamepadrb", "GamepadRB", PadInput::Button(0x0200)),
    ("gamepadback", "GamepadBack", PadInput::Button(0x0020)),
    ("gamepadstart", "GamepadStart", PadInput::Button(0x0010)),
    ("gamepadls", "GamepadLS", PadInput::Button(0x0040)),
    ("gamepadrs", "GamepadRS", PadInput::Button(0x0080)),
    ("gamepaddpadup", "GamepadDpadUp", PadInput::Button(0x0001)),
    ("gamepaddpaddown", "GamepadDpadDown", PadInput::Button(0x0002)),
    ("gamepaddpadleft", "GamepadDpadLeft", PadInput::Button(0x0004)),
    ("gamepaddpadright", "GamepadDpadRight", PadInput::Button(0x0008)),
    ("gamepadlt", "GamepadLT", PadInput::Trigger(PadSide::Left)),
    ("gamepadrt", "GamepadRT", PadInput::Trigger(PadSide::Right)),
];

fn token_to_trigger(token: &str) -> Option<Trigger> {
    if let Some(&(_, code, vk)) = MOUSE_TABLE.iter().find(|(t, ..)| *t == token) {
        return Some(Trigger::Mouse { code, vk });
    }
    if let Some(&(_, code, input)) = GAMEPAD_TABLE.iter().find(|(t, ..)| *t == token) {
        return Some(Trigger::Gamepad { code, input });
    }
    CODE_TABLE
        .iter()
        .find(|(t, ..)| *t == token)
        .map(|&(_, code, sc, extended)| Trigger::Key { code, sc, extended })
}

fn migrate_v1_token(token: &str) -> Option<String> {
    let bytes = token.as_bytes();
    if bytes.len() != 1 {
        return None;
    }
    let c = bytes[0];
    if c.is_ascii_lowercase() {
        return Some(format!("key{token}"));
    }
    if c.is_ascii_digit() {
        return Some(format!("digit{token}"));
    }
    None
}

fn allows_bare(trigger: &Trigger) -> bool {
    match trigger {
        Trigger::Mouse { .. } => true,
        // Der findes ingen modifiers paa en controller. Kraevede en gamepad-
        // binding en, kunne den aldrig fyre.
        Trigger::Gamepad { .. } => true,
        Trigger::Key { sc, extended, .. } => !extended && matches!(sc, 0x3B..=0x44 | 0x57 | 0x58),
    }
}

pub trait VkResolver {
    fn layout(&self) -> isize;
    fn resolve(&self, sc: u16, extended: bool, layout: isize) -> u16;
}

static LAYOUT_WARNING: Mutex<Option<String>> = Mutex::new(None);

pub fn take_layout_warning() -> Option<String> {
    LAYOUT_WARNING.lock().ok().and_then(|mut slot| slot.take())
}

fn publish_layout_warning(message: String) {
    if let Ok(mut slot) = LAYOUT_WARNING.lock() {
        *slot = Some(message);
    }
}

#[derive(Debug, Default)]
pub struct TriggerVkCache {
    layout: isize,
    vk: u16,
    resolved: bool,
    warned_layout: isize,
}

/// Hvad poll-loekken skal spoerge om for at afgoere "er triggeren nede?".
///
/// Formen er bevidst: opslaget (layout-afhaengigt, cachet) er skilt fra selve
/// laesningen, saa Win32-kaldet bliver paa ET call-site i poll-loekken og
/// cachen forbliver testbar med `FakeResolver` uden Win32 (Testbarhed-reglen,
/// se `PollSnapshot`). Gjorde vi det her til et `is_trigger_down`, ville
/// baade `GetAsyncKeyState` og `XInputGetState` vandre ind i cachen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerProbe {
    /// Laeses med `GetAsyncKeyState`. `0` = uoploeselig paa dette layout.
    Vk(u16),
    /// Laeses af den delte `PadState`.
    Pad(PadInput),
}

/// Oejebliksbillede af pad'en — ren data, saa taerskel- og maske-logikken kan
/// unit-testes uden en fysisk controller. Samme begrundelse som `PollSnapshot`.
///
/// `Default` er "intet nede, ikke tilsluttet", og DET er invarianten der
/// forhindrer et haengende hold: en fejlende laesning skal rapportere alle
/// knapper oppe med det samme, saa kant-detektoren ser en faldende kant og
/// udsender sit `Release`. Backoff maa springe SCANNINGEN over, aldrig niveauet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadState {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub connected: bool,
}

impl PadState {
    pub fn is_down(&self, input: PadInput) -> bool {
        if !self.connected {
            return false;
        }
        match input {
            PadInput::Button(mask) => (self.buttons & mask) != 0,
            PadInput::Trigger(PadSide::Left) => self.left_trigger >= PAD_TRIGGER_THRESHOLD,
            PadInput::Trigger(PadSide::Right) => self.right_trigger >= PAD_TRIGGER_THRESHOLD,
        }
    }
}

impl TriggerVkCache {
    pub fn probe_for(
        &mut self,
        trigger: &Trigger,
        resolver: &dyn VkResolver,
    ) -> (TriggerProbe, Option<String>) {
        let (code, sc, extended) = match trigger {
            Trigger::Mouse { vk, .. } => return (TriggerProbe::Vk(*vk), None),
            Trigger::Gamepad { input, .. } => return (TriggerProbe::Pad(*input), None),
            Trigger::Key { code, sc, extended } => (*code, *sc, *extended),
        };
        let layout = resolver.layout();
        if self.resolved && layout == self.layout {
            return (TriggerProbe::Vk(self.vk), None);
        }
        self.layout = layout;
        let vk = resolver.resolve(sc, extended, layout);
        if vk == 0 {
            let warning = if self.warned_layout != layout {
                self.warned_layout = layout;
                Some(format!(
                    "bindingen {code} findes ikke paa det aktive tastaturlayout"
                ))
            } else {
                None
            };
            return (TriggerProbe::Vk(self.vk), warning);
        }
        self.vk = vk;
        self.resolved = true;
        (TriggerProbe::Vk(vk), None)
    }
}

/// Oejebliksbillede af den fysiske tilstand — ren data, saa kant-logikken kan
/// unit-testes uden Win32 (Testbarhed-reglen).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollSnapshot {
    /// Ctrl ELLER Win — spejler ptt.ts (e.ctrlKey || e.metaKey) saa
    /// CmdOrCtrl-acceleratorer matcher ens i begge lag.
    pub ctrl_or_meta: bool,
    pub shift: bool,
    pub alt: bool,
    pub right_alt: bool,
    pub trigger_down: bool,
    pub window_focused: bool,
}

impl PollSnapshot {
    /// Modifier-/fokus-delen er faelles for alle slots; kun triggeren er
    /// pr. slot. Se `read_shared_snapshot`.
    pub fn with_trigger(self, trigger_down: bool) -> Self {
        PollSnapshot {
            trigger_down,
            ..self
        }
    }
}

/// Kant-detektion af niveau (redapting-porten). Hold-tilstanden foelger den
/// FYSISKE trigger-tast; kombo + fokus evalueres NIVEAU-baseret gennem hele
/// det uarmerede hold — IKKE kun i kant-pollet (fix 2026-07-20: ved samtidigt
/// tryk kan triggeren lande i async-tabellen FOER modifiers; kant-tidspunkts-
/// sampling konsumerede kanten som None, og holdet var doedt indtil fysisk
/// re-tryk — forlaeggets detektor er tilsvarende niveau-komponeret).
/// Konsekvenser (testet nedenfor):
/// - mistet keyup kan aldrig haenge — naeste poll laeser niveauet
/// - modifiers der lander polls efter triggeren fanges stadig (samme hold)
/// - fokus-ankomst midt i et hold fyrer ikke (holdet braendes ved unfocused
///   kombo-fuldendelse og heles foerst ved trigger-slip)
/// - modifier-vrid midt i et ARMERET hold dobbelt-fyrer ikke
#[derive(Debug, Default)]
pub struct ComboEdgeDetector {
    trigger_held: bool,
    armed: bool,
    /// Sat naar komboen fuldendte ufokuseret: dette trigger-hold maa aldrig
    /// armere (og diagnostikken logges kun een gang). Ryddes ved trigger-slip.
    burned: bool,
    altgr_logged: bool,
    disqualified_logged: bool,
    suppress_until_keyup: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    None,
    Press,
    Release,
    /// Fuldt kombo-tryk set, men canvas var ikke forgrundsvindue: kanten er
    /// bevidst kasseret (vindues-lokal hotkey). Skilles fra None saa polleren
    /// kan logge det som "doede tryk"-evidens — ser brugeren et dødt tryk og
    /// loggen viser denne, er fokus-gaten synderen; er loggen tavs, sluges
    /// trykket i JS-laget.
    SuppressedUnfocused,
    SuppressedUnfocusedQuiet,
    SuppressedAltGr,
    SuppressedModifierDown,
}

/// Modifier-reglen — SUBSET-match, ikke exact: en kombo kraever de modifiers
/// den navngiver, og er ligeglad med resten. En BAR kombo (ingen modifiers)
/// kraever derimod at INGEN modifier er nede.
///
/// Reglen er normativ og skal staa i lockstep med `ptt.ts::matchesAccelerator`
/// paa den anden side af sproggraensen (se modulets hoved-kommentar). Netop
/// derfor bor den ét sted her: `step` evaluerede den tidligere to gange —
/// én gang i AltGr-grenen og én gang i den normale — med to identiske kopier
/// tyve linjer fra hinanden.
fn combo_matches(combo: &KeyCombo, snapshot: &PollSnapshot) -> bool {
    if combo.is_bare() {
        !snapshot.ctrl_or_meta && !snapshot.shift && !snapshot.alt
    } else {
        (!combo.ctrl || snapshot.ctrl_or_meta)
            && (!combo.shift || snapshot.shift)
            && (!combo.alt || snapshot.alt)
    }
}

/// Kan ÉT fysisk tastetryk fyre begge genveje?
///
/// Reglen foelger direkte af `combo_matches`' subset-semantik, og den er
/// derfor IKKE "er de to ens": `Ctrl+Space` og `Ctrl+Shift+Space` er
/// forskellige `KeyCombo`er, men et Ctrl+Shift+Space-tryk matcher dem BEGGE —
/// den foerste kraever kun at Ctrl er nede og er ligeglad med Shift.
///
/// | Samme trigger | Modifiers                | Kolliderer |
/// |---|---|---|
/// | nej | — | nej |
/// | ja | begge bare | ja (de er identiske) |
/// | ja | begge navngiver modifiers | ja — unionen af dem matcher begge |
/// | ja | een bar, een med modifiers | nej — en bar kombo kraever at INGEN modifier er nede |
///
/// Sammenfattet: to genveje kolliderer praecis naar de deler trigger-tast og
/// er enige om hvorvidt de har modifiers. Det er ogsaa den formulering
/// brugeren faar at se — "de to genveje maa ikke bruge samme tast".
pub fn collides(a: &KeyCombo, b: &KeyCombo) -> bool {
    a.trigger == b.trigger && a.is_bare() == b.is_bare()
}

impl ComboEdgeDetector {
    pub fn reset(&mut self) {
        self.trigger_held = false;
        self.armed = false;
        self.burned = false;
        self.altgr_logged = false;
        self.disqualified_logged = false;
        self.suppress_until_keyup = true;
    }

    /// Returnerer den press/release-kant der skal emitteres nu, hvis nogen.
    pub fn step(&mut self, combo: &KeyCombo, snapshot: &PollSnapshot) -> Step {
        if self.suppress_until_keyup {
            self.trigger_held = snapshot.trigger_down;
            if !snapshot.trigger_down {
                self.suppress_until_keyup = false;
            }
            return Step::None;
        }

        let rising = !self.trigger_held && snapshot.trigger_down;
        let falling = self.trigger_held && !snapshot.trigger_down;
        self.trigger_held = snapshot.trigger_down;
        if falling {
            self.burned = false;
            self.altgr_logged = false;
            self.disqualified_logged = false;
            if self.armed {
                self.armed = false;
                return Step::Release;
            }
            return Step::None;
        }
        if !snapshot.trigger_down || self.armed || self.burned {
            return Step::None;
        }
        let bare = combo.is_bare();
        if snapshot.right_alt {
            let would_have_matched = combo_matches(combo, snapshot);
            if would_have_matched && snapshot.window_focused && !self.altgr_logged {
                self.altgr_logged = true;
                return Step::SuppressedAltGr;
            }
            return Step::None;
        }
        if bare && !rising {
            return Step::None;
        }
        if !combo_matches(combo, snapshot) {
            if bare && !self.disqualified_logged {
                self.disqualified_logged = true;
                return Step::SuppressedModifierDown;
            }
            return Step::None;
        }
        if !snapshot.window_focused {
            self.burned = true;
            return if bare {
                Step::SuppressedUnfocusedQuiet
            } else {
                Step::SuppressedUnfocused
            };
        }
        self.armed = true;
        Step::Press
    }
}

// --- Slots ------------------------------------------------------------------

/// Antal uafhaengige genveje polleren holder.
pub const SLOT_COUNT: usize = 2;

/// De to genveje polleren kan fyre. Hver slot har sin EGEN kant-detektor,
/// vk-cache og versions-taeller — de deler kun mikrofonen, og det ejerskab
/// afgoeres i frontenden, ikke her.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeySlot {
    /// Push-to-talk: hele stemme-pipelinen (STT -> router -> dispatch).
    Ptt,
    /// Diktering: STT direkte ind i det fokuserede korts composer.
    Dictation,
}

impl HotkeySlot {
    pub const ALL: [HotkeySlot; SLOT_COUNT] = [HotkeySlot::Ptt, HotkeySlot::Dictation];

    fn index(self) -> usize {
        self as usize
    }

    /// Wire-navnet i `WAKE_EVENT`-payloaden. Frontenden router paa det, saa
    /// vaerdierne er kontrakt: aendres de, holder App.tsx op med at hoere efter.
    pub fn wire_name(self) -> &'static str {
        match self {
            HotkeySlot::Ptt => "ptt",
            HotkeySlot::Dictation => "dictation",
        }
    }

    /// Brugervendt dansk navn — bruges i layout-advarslen, som naar helt ud i
    /// indstillingerne via `settings_warning`.
    fn label(self) -> &'static str {
        match self {
            HotkeySlot::Ptt => "stemme-aktiveringen",
            HotkeySlot::Dictation => "dikteringen",
        }
    }
}

// --- Delt tilstand (kommando-traad skriver, poller-traad laeser) ------------

static COMBOS: Mutex<[Option<KeyCombo>; SLOT_COUNT]> = Mutex::new([None; SLOT_COUNT]);
/// Bumpes ved hver accelerator-aendring: polleren nulstiller sin kant-tilstand
/// saa et hold paatvunget over en konfigurations-aendring ikke fyrer forkert.
///
/// EEN TAELLER PR. SLOT, og det er ikke kosmetik: `reset()` saetter
/// `suppress_until_keyup`, saa en faelles taeller ville lade en aendring af den
/// ene genvej sluge release-kanten paa et IGANGVAERENDE hold i den anden.
/// Frontenden ville aldrig faa sit release, og mikrofonen stod aaben.
static COMBO_VERSIONS: [AtomicU64; SLOT_COUNT] = [AtomicU64::new(0), AtomicU64::new(0)];
/// Faelles for begge slots: `HotkeyRecorder` suspenderer mens brugeren optager
/// en NY genvej, og da skal ingen af dem fyre.
static SUSPENDED: AtomicBool = AtomicBool::new(false);
/// Talminal-vinduets HWND (sat ved setup). Fokus-gaten sammenligner det med
/// GetForegroundWindow pr. poll — 0 = ukendt = gate lukket (fail-closed).
static WINDOW_HWND: AtomicIsize = AtomicIsize::new(0);
static POLLER_STARTED: AtomicBool = AtomicBool::new(false);

/// Saet (eller udskift) en slots accelerator. Parse-fejl rapporteres til
/// kalderen (frontenden viser den i HUD'et) — aldrig lydloes (redapting
/// review-fund #17/#47: "hotkeys virker bare ikke" uden fejl er usynligt).
///
/// At suspensionen ryddes er BEVIDST (testet nedenfor): kommandoen kaldes ved
/// hver mount, og det er den vej en suspension der blev haengende — fordi
/// indstillingsvinduet forsvandt midt i en optagelse — bliver helet igen.
pub fn set_accelerator(slot: HotkeySlot, accel: &str) -> Result<(), String> {
    let combo = parse_accelerator(accel)?;
    let mut slots = COMBOS.lock().map_err(|e| e.to_string())?;
    slots[slot.index()] = Some(combo);
    COMBO_VERSIONS[slot.index()].fetch_add(1, Ordering::Release);
    SUSPENDED.store(false, Ordering::Release);
    Ok(())
}

pub fn set_suspended(suspended: bool) {
    SUSPENDED.store(suspended, Ordering::Release);
    // Begge detektorer nulstilles: et hold der spaender hen over suspensionen
    // maa ikke fyre naar den ophaeves — uanset hvilken genvej det var.
    for version in &COMBO_VERSIONS {
        version.fetch_add(1, Ordering::Release);
    }
}

pub fn is_suspended() -> bool {
    SUSPENDED.load(Ordering::Acquire)
}

/// Registrér canvas-vinduets HWND (kaldes ved setup). Polleren maaler fokus
/// mod det — niveau-baseret, aldrig via fokus-events.
pub fn set_window_hwnd(hwnd: isize) {
    WINDOW_HWND.store(hwnd, Ordering::Release);
}

#[cfg(windows)]
fn is_vk_down(vk: u16) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    (unsafe { GetAsyncKeyState(i32::from(vk)) } as u16 & 0x8000) != 0
}

/// Hvor mange polls der springes over efter en mislykket pad-scanning.
/// 20 × 5 ms = 100 ms, hvilket er den oevre graense for hvor sent et foerste
/// tryk kan blive set efter at controlleren har sovet. PTT er et HOLD — man
/// trykker og taler bagefter — saa 100 ms er umaerkeligt, mens det skaerer
/// enhedsopdagelsen fra 200 til 10 opslag i sekundet.
#[cfg(windows)]
const PAD_RESCAN_POLLS: u32 = 20;

#[cfg(windows)]
fn read_pad_state(index: u32) -> PadState {
    use windows_sys::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};
    let mut state: XINPUT_STATE = unsafe { std::mem::zeroed() };
    // ERROR_SUCCESS == 0. Alt andet (i praksis ERROR_DEVICE_NOT_CONNECTED)
    // giver `default()` = intet nede — se invarianten paa `PadState`.
    if unsafe { XInputGetState(index, &mut state) } != 0 {
        return PadState::default();
    }
    PadState {
        buttons: state.Gamepad.wButtons,
        left_trigger: state.Gamepad.bLeftTrigger,
        right_trigger: state.Gamepad.bRightTrigger,
        connected: true,
    }
}

/// Laeser slot 0 med backoff naar der ingen pad er. Virtual Desktop leverer
/// én emuleret pad, saa slot 1-3 spoerges aldrig.
#[cfg(windows)]
#[derive(Default)]
struct PadReader {
    skip_polls: u32,
}

#[cfg(windows)]
impl PadReader {
    /// `needed` er falsk naar ingen slot har en gamepad-binding — da kaldes
    /// XInput ALDRIG, og brugere uden controller betaler intet.
    fn read(&mut self, needed: bool) -> PadState {
        if !needed {
            self.skip_polls = 0;
            return PadState::default();
        }
        if self.skip_polls > 0 {
            self.skip_polls -= 1;
            return PadState::default();
        }
        let state = read_pad_state(0);
        if !state.connected {
            self.skip_polls = PAD_RESCAN_POLLS;
        }
        state
    }
}

/// Niveau-baseret fokus: er canvas forgrundsvinduet LIGE NU? Laeses friskt
/// pr. poll ligesom tasterne — et tabt/omrokeret Focused-event kan aldrig
/// wedge gaten (naeste poll laeser bare sandheden). GetForegroundWindow
/// returnerer altid top-level-vinduet, saa WebView2'ens child-HWND-fokus
/// forstyrrer ikke sammenligningen.
#[cfg(windows)]
fn foreground_is_canvas() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let canvas = WINDOW_HWND.load(Ordering::Acquire);
    if canvas == 0 {
        return false;
    }
    (unsafe { GetForegroundWindow() }) as isize == canvas
}

#[cfg(windows)]
struct Win32Resolver;

#[cfg(windows)]
impl VkResolver for Win32Resolver {
    fn layout(&self) -> isize {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        let hwnd = WINDOW_HWND.load(Ordering::Acquire);
        let tid = unsafe { GetWindowThreadProcessId(hwnd as _, std::ptr::null_mut()) };
        unsafe { GetKeyboardLayout(tid) as isize }
    }

    fn resolve(&self, sc: u16, extended: bool, layout: isize) -> u16 {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MapVirtualKeyExW, MAPVK_VSC_TO_VK_EX,
        };
        let scan = if extended {
            0xE000u32 | u32::from(sc)
        } else {
            u32::from(sc)
        };
        (unsafe { MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK_EX, layout as _) }) as u16
    }
}

/// Modifier- og fokus-delen, som ALLE slots deler. Laeses een gang pr. poll:
/// med to slots ville et snapshot pr. slot koste seks ekstra GetAsyncKeyState
/// og et ekstra GetForegroundWindow hvert 5. ms uden at kunne give et andet
/// svar — de fysiske modifiers er jo de samme for begge genveje.
/// `trigger_down` staar `false` her og saettes pr. slot af `with_trigger`.
#[cfg(windows)]
fn read_shared_snapshot() -> PollSnapshot {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_CONTROL, VK_LWIN, VK_MENU, VK_RMENU, VK_RWIN, VK_SHIFT,
    };
    PollSnapshot {
        ctrl_or_meta: is_vk_down(VK_CONTROL) || is_vk_down(VK_LWIN) || is_vk_down(VK_RWIN),
        shift: is_vk_down(VK_SHIFT),
        alt: is_vk_down(VK_MENU),
        right_alt: is_vk_down(VK_RMENU),
        trigger_down: false,
        window_focused: foreground_is_canvas(),
    }
}

/// Start polleren (idempotent). Emitter WAKE_EVENT ved hver godkendt kant;
/// frontenden ejer al videre semantik (session-togglen).
pub fn spawn_wake_poller(app: AppHandle) {
    if POLLER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(not(windows))]
    {
        let _ = app;
    }
    #[cfg(windows)]
    thread::Builder::new()
        .name("canvas-wake-hotkey-poller".to_string())
        .spawn(move || {
            // Haev timer-oploesningen for HELE poller-traadens levetid —
            // uden den er thread::sleep(5) reelt 15,6+ ms (Windows' default
            // granularitet), og korte tap falder mellem polls. Kaldes een
            // gang; traaden lever til process-exit, saa timeEndPeriod
            // behoeves ikke.
            unsafe {
                windows_sys::Win32::Media::timeBeginPeriod(1);
            }
            let mut detectors: [ComboEdgeDetector; SLOT_COUNT] = Default::default();
            let mut vk_caches: [TriggerVkCache; SLOT_COUNT] = Default::default();
            let mut pad_reader = PadReader::default();
            let mut seen_versions =
                HotkeySlot::ALL.map(|slot| COMBO_VERSIONS[slot.index()].load(Ordering::Acquire));
            loop {
                thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
                // Versions-tjekket ligger FOER suspensions-gaten, praecis som
                // da der kun var een slot: `set_suspended` bumper selv, saa
                // nulstillingen skal naa detektoren ogsaa naar vi er tavse.
                for slot in HotkeySlot::ALL {
                    let i = slot.index();
                    let version = COMBO_VERSIONS[i].load(Ordering::Acquire);
                    if version != seen_versions[i] {
                        seen_versions[i] = version;
                        detectors[i].reset();
                        vk_caches[i] = TriggerVkCache::default();
                    }
                }
                if SUSPENDED.load(Ordering::Acquire) {
                    continue;
                }
                let combos = match COMBOS.lock() {
                    Ok(slots) => *slots,
                    Err(_) => continue,
                };
                if combos.iter().all(Option::is_none) {
                    continue;
                }
                let shared = read_shared_snapshot();
                // Pad'en laeses EEN gang pr. poll og deles mellem slots, af
                // samme grund som modifiers deles (se `read_shared_snapshot`):
                // to slots kan ikke give to forskellige svar om den samme
                // fysiske controller.
                let needs_pad = combos
                    .iter()
                    .flatten()
                    .any(|c| matches!(c.trigger, Trigger::Gamepad { .. }));
                let pad = pad_reader.read(needs_pad);
                for slot in HotkeySlot::ALL {
                    let i = slot.index();
                    let Some(combo) = combos[i] else { continue };
                    let (probe, warning) = vk_caches[i].probe_for(&combo.trigger, &Win32Resolver);
                    if let Some(warning) = warning {
                        let named = format!("{}: {warning}", slot.label());
                        eprintln!("[wake-hotkey/{}] {warning}", slot.wire_name());
                        publish_layout_warning(named);
                    }
                    let trigger_down = match probe {
                        TriggerProbe::Vk(vk) => vk != 0 && is_vk_down(vk),
                        TriggerProbe::Pad(input) => pad.is_down(input),
                    };
                    let snapshot = shared.with_trigger(trigger_down);
                    match detectors[i].step(&combo, &snapshot) {
                        Step::Press => {
                            crate::perf_mark_background!(
                                "voice.ptt.press_sampled",
                                serde_json::json!({
                                    "poll_interval_ms": POLL_INTERVAL_MS,
                                    "slot": slot.wire_name(),
                                }),
                            );
                            let _ = app.emit(
                                WAKE_EVENT,
                                WakeHotkeyEvent {
                                    combo: slot.wire_name(),
                                    edge: "press",
                                },
                            );
                        }
                        Step::Release => {
                            crate::perf_mark_background!(
                                "voice.ptt.release_sampled",
                                serde_json::json!({
                                    "poll_interval_ms": POLL_INTERVAL_MS,
                                    "slot": slot.wire_name(),
                                }),
                            );
                            let _ = app.emit(
                                WAKE_EVENT,
                                WakeHotkeyEvent {
                                    combo: slot.wire_name(),
                                    edge: "release",
                                },
                            );
                        }
                        Step::SuppressedUnfocused => {
                            // Doede-tryk-diagnostik: rigtigt kombo-tryk, gate
                            // lukket. Ses denne samtidig med at brugeren kigger
                            // paa canvas, er fokus-maalingen forkert.
                            eprintln!(
                                "[wake-hotkey/{}] kombo-tryk set, men canvas er ikke forgrundsvindue — kanten kasseret",
                                slot.wire_name()
                            );
                        }
                        Step::SuppressedAltGr => {
                            eprintln!(
                                "[wake-hotkey/{}] kombo-tryk set, men hoejre Alt (AltGr) er nede — kanten kasseret",
                                slot.wire_name()
                            );
                        }
                        Step::SuppressedModifierDown => {
                            eprintln!(
                                "[wake-hotkey/{}] bar binding: trigger nede, men en modifier diskvalificerede det eksakte match",
                                slot.wire_name()
                            );
                        }
                        Step::SuppressedUnfocusedQuiet | Step::None => {}
                    }
                }
            }
        })
        .expect("wake-hotkey poller thread");
}

#[derive(Clone, serde::Serialize)]
struct WakeHotkeyEvent {
    combo: &'static str,
    edge: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo() -> KeyCombo {
        parse_accelerator("CmdOrCtrl+Shift+Space").expect("parse")
    }

    fn combo_down() -> PollSnapshot {
        PollSnapshot {
            ctrl_or_meta: true,
            shift: true,
            alt: false,
            right_alt: false,
            trigger_down: true,
            window_focused: true,
        }
    }

    fn all_up() -> PollSnapshot {
        PollSnapshot {
            window_focused: true,
            ..PollSnapshot::default()
        }
    }

    #[test]
    fn parser_accepts_the_default_accelerator() {
        let combo = combo();
        assert!(combo.ctrl && combo.shift && !combo.alt);
        assert_eq!(combo.trigger.code(), "Space");
    }

    #[test]
    fn parser_matches_frontend_grammar() {
        assert_eq!(
            parse_accelerator("Ctrl+Alt+F5").unwrap().trigger.code(),
            "F5"
        );
        assert_eq!(parse_accelerator("Shift+a").unwrap().trigger.code(), "KeyA");
        assert_eq!(
            parse_accelerator("control+9").unwrap().trigger.code(),
            "Digit9"
        );
        assert!(parse_accelerator("Option+Space").unwrap().alt);
    }

    #[test]
    fn parser_rejects_what_the_frontend_rejects() {
        for bad in ["Space", "Ctrl+Foo", "Ctrl+A+B", ""] {
            assert!(parse_accelerator(bad).is_err(), "burde afvise: {bad}");
        }
        assert!(parse_accelerator("F5").is_ok());
        assert!(parse_accelerator("Shift+Escape").is_ok());
    }

    #[test]
    fn parser_matcher_den_delte_fixture() {
        #[derive(serde::Deserialize)]
        struct ValidCase {
            accel: String,
            ctrl: bool,
            shift: bool,
            alt: bool,
            code: String,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            valid: Vec<ValidCase>,
            invalid: Vec<String>,
        }
        let raw = include_str!("../../src/voice/hotkey-grammar.fixtures.json");
        let fixture: Fixture = serde_json::from_str(raw).expect("fixture parser");
        for case in &fixture.valid {
            let combo = parse_accelerator(&case.accel)
                .unwrap_or_else(|e| panic!("{} burde vaere gyldig: {e}", case.accel));
            assert_eq!(combo.ctrl, case.ctrl, "ctrl for {}", case.accel);
            assert_eq!(combo.shift, case.shift, "shift for {}", case.accel);
            assert_eq!(combo.alt, case.alt, "alt for {}", case.accel);
            assert_eq!(combo.trigger.code(), case.code, "code for {}", case.accel);
        }
        for accel in &fixture.invalid {
            assert!(parse_accelerator(accel).is_err(), "{accel} burde afvises");
        }
    }

    #[test]
    fn bare_bindinger_kun_for_f_taster_og_mus() {
        assert!(parse_accelerator("F9").unwrap().is_bare());
        assert!(parse_accelerator("Mouse4").unwrap().is_bare());
        assert!(!parse_accelerator("Ctrl+Shift+Space").unwrap().is_bare());
        assert!(parse_accelerator("KeyA").is_err());
        assert!(parse_accelerator("Space").is_err());
    }

    #[test]
    fn scancode_tabellen_er_korrekt_og_entydig() {
        let cases: &[(&str, u16, bool)] = &[
            ("Ctrl+Semicolon", 0x27, false),
            ("Ctrl+Quote", 0x28, false),
            ("F11", 0x57, false),
            ("F12", 0x58, false),
            ("Ctrl+Home", 0x47, true),
            ("Ctrl+ArrowUp", 0x48, true),
            ("Ctrl+KeyA", 0x1E, false),
            ("Ctrl+Digit0", 0x0B, false),
        ];
        for (accel, sc, extended) in cases {
            match parse_accelerator(accel).unwrap().trigger {
                Trigger::Key {
                    sc: got_sc,
                    extended: got_ext,
                    ..
                } => {
                    assert_eq!((got_sc, got_ext), (*sc, *extended), "{accel}")
                }
                Trigger::Mouse { .. } | Trigger::Gamepad { .. } => {
                    panic!("{accel} burde vaere en tast")
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        for (_, code, sc, extended) in CODE_TABLE {
            assert!(seen.insert((*sc, *extended)), "dublet scancode for {code}");
        }
    }

    #[test]
    fn key_combo_forbliver_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<KeyCombo>();
        assert_copy::<Trigger>();
    }

    struct FakeResolver {
        layout: std::cell::Cell<isize>,
        map: std::cell::RefCell<std::collections::HashMap<(u16, bool), u16>>,
    }

    impl VkResolver for FakeResolver {
        fn layout(&self) -> isize {
            self.layout.get()
        }
        fn resolve(&self, sc: u16, extended: bool, _layout: isize) -> u16 {
            *self.map.borrow().get(&(sc, extended)).unwrap_or(&0)
        }
    }

    #[test]
    fn vk_cache_genoploeser_ved_layout_skift() {
        let r = FakeResolver {
            layout: std::cell::Cell::new(1),
            map: std::cell::RefCell::new([((0x27, false), 0xBA)].into_iter().collect()),
        };
        let trigger = parse_accelerator("Ctrl+Semicolon").unwrap().trigger;
        let mut cache = TriggerVkCache::default();
        assert_eq!(cache.probe_for(&trigger, &r).0, TriggerProbe::Vk(0xBA));
        r.layout.set(2);
        r.map.borrow_mut().insert((0x27, false), 0xC0);
        assert_eq!(cache.probe_for(&trigger, &r).0, TriggerProbe::Vk(0xC0));
    }

    #[test]
    fn vk_cache_beholder_sidste_gyldige_ved_nulretur_og_advarer_een_gang() {
        let r = FakeResolver {
            layout: std::cell::Cell::new(1),
            map: std::cell::RefCell::new([((0x56, false), 0xE2)].into_iter().collect()),
        };
        let trigger = parse_accelerator("Ctrl+IntlBackslash").unwrap().trigger;
        let mut cache = TriggerVkCache::default();
        assert_eq!(cache.probe_for(&trigger, &r).0, TriggerProbe::Vk(0xE2));
        r.layout.set(2);
        r.map.borrow_mut().clear();
        let (probe, warning) = cache.probe_for(&trigger, &r);
        assert_eq!(probe, TriggerProbe::Vk(0xE2));
        assert!(warning.expect("advarsel").contains("IntlBackslash"));
        assert_eq!(cache.probe_for(&trigger, &r).1, None);
    }

    #[test]
    fn vk_cache_springer_oploesning_over_for_mus() {
        let r = FakeResolver {
            layout: std::cell::Cell::new(1),
            map: std::cell::RefCell::new(std::collections::HashMap::new()),
        };
        let trigger = parse_accelerator("Mouse4").unwrap().trigger;
        let mut cache = TriggerVkCache::default();
        assert_eq!(cache.probe_for(&trigger, &r), (TriggerProbe::Vk(0x05), None));
    }

    /// Gamepad'en maa aldrig roere layout-oploesningen: `FakeResolver` her har
    /// et TOMT kort, saa ethvert opslag ville give VK 0 og en advarsel.
    #[test]
    fn vk_cache_springer_oploesning_over_for_gamepad() {
        let r = FakeResolver {
            layout: std::cell::Cell::new(1),
            map: std::cell::RefCell::new(std::collections::HashMap::new()),
        };
        let trigger = parse_accelerator("GamepadLT").unwrap().trigger;
        let mut cache = TriggerVkCache::default();
        assert_eq!(
            cache.probe_for(&trigger, &r),
            (TriggerProbe::Pad(PadInput::Trigger(PadSide::Left)), None)
        );
    }

    #[test]
    fn gamepad_bindinger_maa_staa_bare() {
        assert!(parse_accelerator("GamepadLT").unwrap().is_bare());
        assert!(parse_accelerator("GamepadA").unwrap().is_bare());
        assert!(parse_accelerator("Ctrl+GamepadLT").is_ok());
    }

    /// De to analoge triggere baerer samme slags `PadInput` og ville kollidere
    /// hvis `code` ikke var et felt paa varianten — og saa kunne LT og RT ikke
    /// sidde i hver sin slot.
    #[test]
    fn lt_og_rt_kolliderer_ikke() {
        let lt = parse_accelerator("GamepadLT").unwrap();
        let rt = parse_accelerator("GamepadRT").unwrap();
        assert!(!collides(&lt, &rt));
        assert!(collides(&lt, &parse_accelerator("GamepadLT").unwrap()));
    }

    #[test]
    fn gamepad_tabellen_er_entydig() {
        let mut codes = std::collections::HashSet::new();
        let mut inputs = std::collections::HashSet::new();
        for (token, code, input) in GAMEPAD_TABLE {
            assert!(codes.insert(*code), "dublet code {code}");
            let key = format!("{input:?}");
            assert!(inputs.insert(key), "dublet input for {code}");
            assert_eq!(*token, code.to_lowercase(), "token/code i utakt: {code}");
        }
        assert_eq!(GAMEPAD_TABLE.len(), 16, "14 bit-flag + 2 analoge triggere");
    }

    /// Maskerne er skrevet ud i haanden for at holde tabellen platform-fri.
    /// Denne test er prisen for det: den laaser dem mod windows-sys' egne
    /// konstanter, saa en tastefejl i et hex-tal ikke bliver en binding der
    /// tavst fyrer paa den forkerte knap.
    #[cfg(windows)]
    #[test]
    fn masker_matcher_xinput() {
        use windows_sys::Win32::UI::Input::XboxController::*;
        let expected: &[(&str, u16)] = &[
            ("GamepadA", XINPUT_GAMEPAD_A),
            ("GamepadB", XINPUT_GAMEPAD_B),
            ("GamepadX", XINPUT_GAMEPAD_X),
            ("GamepadY", XINPUT_GAMEPAD_Y),
            ("GamepadLB", XINPUT_GAMEPAD_LEFT_SHOULDER),
            ("GamepadRB", XINPUT_GAMEPAD_RIGHT_SHOULDER),
            ("GamepadBack", XINPUT_GAMEPAD_BACK),
            ("GamepadStart", XINPUT_GAMEPAD_START),
            ("GamepadLS", XINPUT_GAMEPAD_LEFT_THUMB),
            ("GamepadRS", XINPUT_GAMEPAD_RIGHT_THUMB),
            ("GamepadDpadUp", XINPUT_GAMEPAD_DPAD_UP),
            ("GamepadDpadDown", XINPUT_GAMEPAD_DPAD_DOWN),
            ("GamepadDpadLeft", XINPUT_GAMEPAD_DPAD_LEFT),
            ("GamepadDpadRight", XINPUT_GAMEPAD_DPAD_RIGHT),
        ];
        for (code, mask) in expected {
            let found = GAMEPAD_TABLE
                .iter()
                .find(|(_, c, _)| c == code)
                .unwrap_or_else(|| panic!("{code} mangler i GAMEPAD_TABLE"));
            assert_eq!(found.2, PadInput::Button(*mask), "maske for {code}");
        }
    }

    /// Taerskel-logikken, testet uden en fysisk controller — hele pointen med
    /// at `PadState` er ren data.
    #[test]
    fn pad_taerskel_og_maske() {
        let mut pad = PadState {
            buttons: 0x1000,
            left_trigger: PAD_TRIGGER_THRESHOLD,
            right_trigger: PAD_TRIGGER_THRESHOLD - 1,
            connected: true,
        };
        assert!(pad.is_down(PadInput::Button(0x1000)));
        assert!(!pad.is_down(PadInput::Button(0x2000)));
        assert!(pad.is_down(PadInput::Trigger(PadSide::Left)));
        assert!(!pad.is_down(PadInput::Trigger(PadSide::Right)));

        // Chromium meldte `pressed` ved 0,25 — for let til at aabne en mikrofon.
        pad.left_trigger = (0.25 * 255.0) as u8;
        assert!(!pad.is_down(PadInput::Trigger(PadSide::Left)));

        // Frakoblet = alt oppe, uanset hvad felterne staar paa. Det er
        // invarianten der forhindrer et haengende hold.
        pad.connected = false;
        pad.left_trigger = 255;
        pad.buttons = 0xFFFF;
        assert!(!pad.is_down(PadInput::Trigger(PadSide::Left)));
        assert!(!pad.is_down(PadInput::Button(0x1000)));
    }

    /// En frakoblet pad midt i et hold skal give et RELEASE, ikke stilhed.
    #[test]
    fn frakoblet_pad_midt_i_hold_slipper() {
        let combo = parse_accelerator("GamepadLT").unwrap();
        let mut detector = ComboEdgeDetector::default();
        let base = PollSnapshot {
            window_focused: true,
            ..PollSnapshot::default()
        };
        // Slip den indledende suppress_until_keyup-gate.
        assert_eq!(detector.step(&combo, &base.with_trigger(false)), Step::None);
        assert_eq!(detector.step(&combo, &base.with_trigger(true)), Step::Press);
        // Pad'en forsvinder: laesningen rapporterer alt oppe.
        assert_eq!(
            detector.step(&combo, &base.with_trigger(false)),
            Step::Release
        );
    }

    #[test]
    fn press_then_release_cycle() {
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        assert_eq!(detector.step(&combo, &combo_down()), Step::Press);
        // Uaendret niveau (auto-repeat/holdet) giver ingen nye kanter.
        assert_eq!(detector.step(&combo, &combo_down()), Step::None);
        assert_eq!(detector.step(&combo, &all_up()), Step::Release);
        assert_eq!(detector.step(&combo, &all_up()), Step::None);
    }

    #[test]
    fn next_press_after_lost_keyup_still_fires() {
        // Redapting-regressionen "foerste tryk er doedt" (2026-07-03): en
        // event-baseret matcher kunne haenge i "kombo er nede" naar keyup gik
        // tabt. Niveau-baseret detektion kan ikke haenge: naeste poll laeser
        // bare sandheden.
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        assert_eq!(detector.step(&combo, &combo_down()), Step::Press);
        // Keyup-EVENTET gik tabt — men polled niveau viser tasten oppe.
        assert_eq!(detector.step(&combo, &all_up()), Step::Release);
        assert_eq!(detector.step(&combo, &combo_down()), Step::Press);
    }

    #[test]
    fn unfocused_press_never_arms_so_no_release() {
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        let unfocused_hold = PollSnapshot {
            window_focused: false,
            ..combo_down()
        };
        // Trykket sker mens et andet vindue har fokus: aldrig wake — men
        // synligt som diagnostik (doede-tryk-evidens).
        assert_eq!(
            detector.step(&combo, &unfocused_hold),
            Step::SuppressedUnfocused
        );
        // Fokus ankommer MIDT i holdet: stadig ingen stigende kant.
        assert_eq!(detector.step(&combo, &combo_down()), Step::None);
        // Et uarmeret hold maa heller ikke udsende release.
        assert_eq!(detector.step(&combo, &all_up()), Step::None);
    }

    #[test]
    fn focus_loss_mid_hold_still_releases() {
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        assert_eq!(detector.step(&combo, &combo_down()), Step::Press);
        let unfocused_release = PollSnapshot {
            window_focused: false,
            ..all_up()
        };
        assert_eq!(detector.step(&combo, &unfocused_release), Step::Release);
    }

    #[test]
    fn extra_modifier_never_disqualifies() {
        // Fantom-modifier-vaernet (2026-07-20, redapting-forlaeggets subset-
        // semantik): en stuck/fantom Alt-bit i async-tabellen maa ALDRIG
        // goere komboen tavst doed — Ctrl+Shift+Alt+Space skal stadig fyre
        // som Ctrl+Shift+Space.
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        let with_alt = PollSnapshot {
            alt: true,
            ..combo_down()
        };
        assert_eq!(detector.step(&combo, &with_alt), Step::Press);
        assert_eq!(detector.step(&combo, &all_up()), Step::Release);
    }

    fn bare_combo() -> KeyCombo {
        parse_accelerator("F9").expect("parse")
    }

    fn bare_down() -> PollSnapshot {
        PollSnapshot {
            trigger_down: true,
            window_focused: true,
            ..PollSnapshot::default()
        }
    }

    #[test]
    fn bar_binding_matcher_eksakt() {
        let mut d = ComboEdgeDetector::default();
        let combo = bare_combo();
        let with_shift = PollSnapshot {
            shift: true,
            ..bare_down()
        };
        assert_eq!(d.step(&combo, &with_shift), Step::SuppressedModifierDown);
        assert_eq!(d.step(&combo, &all_up()), Step::None);
        assert_eq!(d.step(&combo, &bare_down()), Step::Press);
    }

    #[test]
    fn bar_binding_armerer_ikke_ved_at_slippe_en_modifier() {
        let mut d = ComboEdgeDetector::default();
        let combo = bare_combo();
        let with_ctrl = PollSnapshot {
            ctrl_or_meta: true,
            ..bare_down()
        };
        assert_eq!(d.step(&combo, &with_ctrl), Step::SuppressedModifierDown);
        assert_eq!(d.step(&combo, &bare_down()), Step::None);
        assert_eq!(d.step(&combo, &bare_down()), Step::None);
    }

    #[test]
    fn hoejre_alt_blokerer_armering_men_venstre_alt_goer_ikke() {
        let mut d = ComboEdgeDetector::default();
        let combo = combo();
        let with_right_alt = PollSnapshot {
            right_alt: true,
            alt: true,
            ..combo_down()
        };
        assert_eq!(d.step(&combo, &with_right_alt), Step::SuppressedAltGr);
        assert_eq!(d.step(&combo, &with_right_alt), Step::None);
        assert_eq!(d.step(&combo, &all_up()), Step::None);
        let with_left_alt = PollSnapshot {
            alt: true,
            ..combo_down()
        };
        assert_eq!(d.step(&combo, &with_left_alt), Step::Press);
    }

    #[test]
    fn hoejre_alt_midt_i_et_armeret_hold_draeber_ikke_release() {
        let mut d = ComboEdgeDetector::default();
        let combo = combo();
        assert_eq!(d.step(&combo, &combo_down()), Step::Press);
        let brushed = PollSnapshot {
            right_alt: true,
            ..combo_down()
        };
        assert_eq!(d.step(&combo, &brushed), Step::None);
        let released = PollSnapshot {
            right_alt: true,
            ..all_up()
        };
        assert_eq!(d.step(&combo, &released), Step::Release);
    }

    #[test]
    fn bar_binding_ufokuseret_braender_holdet_men_logger_ikke() {
        let mut d = ComboEdgeDetector::default();
        let combo = parse_accelerator("Mouse4").unwrap();
        let unfocused = PollSnapshot {
            window_focused: false,
            ..bare_down()
        };
        assert_eq!(d.step(&combo, &unfocused), Step::SuppressedUnfocusedQuiet);
        assert_eq!(d.step(&combo, &bare_down()), Step::None);
        assert_eq!(d.step(&combo, &all_up()), Step::None);
        assert_eq!(d.step(&combo, &bare_down()), Step::Press);
    }

    #[test]
    fn modifier_drift_mid_hold_keeps_armed() {
        // Holdet foelger den fysiske trigger-tast: slippes Shift midt i
        // holdet og trykkes igen, opstaar ingen ny stigende kant.
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        assert_eq!(detector.step(&combo, &combo_down()), Step::Press);
        let shift_released = PollSnapshot {
            shift: false,
            ..combo_down()
        };
        assert_eq!(detector.step(&combo, &shift_released), Step::None);
        let released_without_modifiers = PollSnapshot {
            shift: false,
            ..all_up()
        };
        assert_eq!(
            detector.step(&combo, &released_without_modifiers),
            Step::Release
        );
    }

    #[test]
    fn bare_trigger_never_fires() {
        // Fund A-regressionen fra redapting: bare mellemrum maa ALDRIG
        // matche komboen — modifiers laeses altid live paa kanten.
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        let bare_space = PollSnapshot {
            ctrl_or_meta: false,
            shift: false,
            ..combo_down()
        };
        assert_eq!(detector.step(&combo, &bare_space), Step::None);
        assert_eq!(detector.step(&combo, &all_up()), Step::None);
    }

    #[test]
    fn bare_trigger_unfocused_is_none_not_diagnostics() {
        // Log-spam-vaernet: mellemrum i en ANDEN app er ikke et kombo-tryk —
        // det maa aldrig rapporteres som SuppressedUnfocused (modifier-match
        // evalueres FOER fokus-gaten).
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        let bare_space_unfocused = PollSnapshot {
            ctrl_or_meta: false,
            shift: false,
            window_focused: false,
            ..combo_down()
        };
        assert_eq!(detector.step(&combo, &bare_space_unfocused), Step::None);
    }

    #[test]
    fn simultaneous_press_where_modifiers_land_late_still_fires() {
        // Dogfood-fund 2026-07-20: ved samtidigt tryk paa alle taster
        // kan triggeren lande i async-tabellen FOER modifiers. Den gamle
        // kant-tidspunkts-sampling konsumerede kanten som None, og holdet
        // var doedt indtil fysisk re-tryk. Niveau-armering: komboen maa
        // fuldende sent i samme hold.
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        let space_landed_first = PollSnapshot {
            ctrl_or_meta: false,
            shift: false,
            ..combo_down()
        };
        assert_eq!(detector.step(&combo, &space_landed_first), Step::None);
        // Naeste poll: modifiers er landet — samme hold fyrer nu.
        assert_eq!(detector.step(&combo, &combo_down()), Step::Press);
        assert_eq!(detector.step(&combo, &all_up()), Step::Release);
    }

    #[test]
    fn unfocused_completion_burns_hold_and_logs_once() {
        // SuppressedUnfocused rapporteres EEN gang pr. hold (burned) — ikke
        // hver poll — og hverken fokus-ankomst eller modifier-genforsoeg i
        // samme hold maa armere. Trigger-slip healer.
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        let unfocused_hold = PollSnapshot {
            window_focused: false,
            ..combo_down()
        };
        assert_eq!(
            detector.step(&combo, &unfocused_hold),
            Step::SuppressedUnfocused
        );
        assert_eq!(detector.step(&combo, &unfocused_hold), Step::None);
        assert_eq!(detector.step(&combo, &combo_down()), Step::None);
        assert_eq!(detector.step(&combo, &all_up()), Step::None);
        assert_eq!(detector.step(&combo, &combo_down()), Step::Press);
    }

    #[test]
    fn reset_while_held_suppresses_until_keyup() {
        let mut detector = ComboEdgeDetector::default();
        let combo = combo();
        assert_eq!(detector.step(&combo, &combo_down()), Step::Press);
        detector.reset();
        assert_eq!(detector.step(&combo, &combo_down()), Step::None);
        assert_eq!(detector.step(&combo, &all_up()), Step::None);
        assert_eq!(detector.step(&combo, &combo_down()), Step::Press);
    }

    #[test]
    fn wake_hotkey_payload_has_combo_and_edge_wire_format() {
        let press = serde_json::to_value(WakeHotkeyEvent {
            combo: HotkeySlot::Ptt.wire_name(),
            edge: "press",
        })
        .expect("serialize press payload");
        let release = serde_json::to_value(WakeHotkeyEvent {
            combo: HotkeySlot::Dictation.wire_name(),
            edge: "release",
        })
        .expect("serialize release payload");
        assert_eq!(press, serde_json::json!({"combo": "ptt", "edge": "press"}));
        assert_eq!(
            release,
            serde_json::json!({"combo": "dictation", "edge": "release"})
        );
    }

    /// COMBOS/COMBO_VERSIONS/SUSPENDED er proces-globale, og cargo koerer
    /// tests i traade i SAMME proces. Uden en faelles laas ville testene
    /// nedenfor lekke ind i hinanden — praecis den slags flakiness der er
    /// dyrest at fejlfinde. (Datamappe-vagten loeser det samme problem for
    /// integrationstestene med `common::serial()`.)
    static GLOBAL_STATE: Mutex<()> = Mutex::new(());

    fn with_global_state<T>(body: impl FnOnce() -> T) -> T {
        let guard = GLOBAL_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let out = body();
        set_accelerator(HotkeySlot::Ptt, crate::workspace::DEFAULT_PTT_HOTKEY).expect("reset ptt");
        set_accelerator(
            HotkeySlot::Dictation,
            crate::workspace::DEFAULT_DICTATION_HOTKEY,
        )
        .expect("reset dictation");
        drop(guard);
        out
    }

    #[test]
    fn set_accelerator_rejects_parse_errors() {
        with_global_state(|| {
            assert!(set_accelerator(HotkeySlot::Ptt, "ikke-en-accelerator").is_err());
            assert!(set_accelerator(HotkeySlot::Ptt, "Ctrl+Shift+Space").is_ok());
            assert!(set_accelerator(HotkeySlot::Dictation, "Ctrl+Shift+KeyD").is_ok());
        });
    }

    #[test]
    fn de_to_slots_holder_hver_sin_kombo() {
        with_global_state(|| {
            set_accelerator(HotkeySlot::Ptt, "Ctrl+Shift+Space").expect("ptt");
            set_accelerator(HotkeySlot::Dictation, "Ctrl+Shift+KeyD").expect("dictation");
            let slots = *COMBOS.lock().expect("lock");
            assert_eq!(
                slots[HotkeySlot::Ptt.index()]
                    .expect("ptt-kombo")
                    .trigger
                    .code(),
                "Space"
            );
            assert_eq!(
                slots[HotkeySlot::Dictation.index()]
                    .expect("dikterings-kombo")
                    .trigger
                    .code(),
                "KeyD"
            );
        });
    }

    #[test]
    fn versionstaelleren_er_pr_slot() {
        // Regressionsvaern: med EEN faelles taeller ville en aendring af den
        // ene genvej kalde reset() paa den ANDENS detektor. reset() saetter
        // suppress_until_keyup, saa et igangvaerende hold ville miste sin
        // release-kant — frontenden fik aldrig sit release, og mikrofonen
        // stod aaben indtil brugeren trykkede forfra.
        with_global_state(|| {
            let before = HotkeySlot::ALL.map(|s| COMBO_VERSIONS[s.index()].load(Ordering::Acquire));
            set_accelerator(HotkeySlot::Ptt, "Alt+KeyQ").expect("ptt");
            let after = HotkeySlot::ALL.map(|s| COMBO_VERSIONS[s.index()].load(Ordering::Acquire));
            assert!(
                after[HotkeySlot::Ptt.index()] > before[HotkeySlot::Ptt.index()],
                "ptt-slotten skal bumpe sin egen taeller"
            );
            assert_eq!(
                after[HotkeySlot::Dictation.index()],
                before[HotkeySlot::Dictation.index()],
                "en ptt-aendring maa ikke nulstille dikterings-detektoren"
            );
        });
    }

    #[test]
    fn suspension_nulstiller_begge_slots() {
        with_global_state(|| {
            let before = HotkeySlot::ALL.map(|s| COMBO_VERSIONS[s.index()].load(Ordering::Acquire));
            set_suspended(true);
            assert!(is_suspended());
            let during = HotkeySlot::ALL.map(|s| COMBO_VERSIONS[s.index()].load(Ordering::Acquire));
            for slot in HotkeySlot::ALL {
                assert!(
                    during[slot.index()] > before[slot.index()],
                    "{} skal nulstilles af en suspension",
                    slot.wire_name()
                );
            }
            // Suspensionen ryddes bevidst af set_accelerator (mount heler et
            // haengende indstillingsvindue) — og nu fra BEGGE kommandoer.
            set_accelerator(HotkeySlot::Dictation, "Alt+KeyQ").expect("set under suspension");
            assert!(!is_suspended());
        });
    }

    #[test]
    fn kollision_er_delt_trigger_ikke_lighed() {
        let p = |a: &str| parse_accelerator(a).expect(a);
        // Subset-fælden: ét Ctrl+Shift+Space-tryk matcher BEGGE.
        assert!(collides(&p("Ctrl+Space"), &p("Ctrl+Shift+Space")));
        assert!(collides(&p("Ctrl+Shift+Space"), &p("Ctrl+Shift+Space")));
        assert!(collides(&p("Ctrl+Space"), &p("Alt+Space")));
        assert!(collides(&p("F9"), &p("F9")));
        // Forskellig trigger: kan aldrig fyre samtidig.
        assert!(!collides(&p("Ctrl+Shift+Space"), &p("Ctrl+Shift+KeyD")));
        assert!(!collides(&p("F9"), &p("F10")));
        // Bar mod ikke-bar paa samme tast: den bare kraever at INGEN modifier
        // er nede, saa intet enkelt tryk kan opfylde dem begge.
        assert!(!collides(&p("F9"), &p("Ctrl+F9")));
    }
}
