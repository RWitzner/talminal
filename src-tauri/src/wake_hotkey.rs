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
}

impl Trigger {
    pub fn code(&self) -> &'static str {
        match self {
            Trigger::Key { code, .. } | Trigger::Mouse { code, .. } => code,
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

/// Parser med SAMME grammatik som ptt.ts::parseAccelerator (kontrakten er
/// `canvas/hotkey-grammar.fixtures.json`, som begge lag tester mod).
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
            "{accel} kraever mindst een modifier — kun F1-F12 og musetaster maa staa alene"
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

fn token_to_trigger(token: &str) -> Option<Trigger> {
    if let Some(&(_, code, vk)) = MOUSE_TABLE.iter().find(|(t, ..)| *t == token) {
        return Some(Trigger::Mouse { code, vk });
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

impl TriggerVkCache {
    pub fn vk_for(
        &mut self,
        trigger: &Trigger,
        resolver: &dyn VkResolver,
    ) -> (u16, Option<String>) {
        let (code, sc, extended) = match trigger {
            Trigger::Mouse { vk, .. } => return (*vk, None),
            Trigger::Key { code, sc, extended } => (*code, *sc, *extended),
        };
        let layout = resolver.layout();
        if self.resolved && layout == self.layout {
            return (self.vk, None);
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
            return (self.vk, warning);
        }
        self.vk = vk;
        self.resolved = true;
        (vk, None)
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

// --- Delt tilstand (kommando-traad skriver, poller-traad laeser) ------------

static COMBO: Mutex<Option<KeyCombo>> = Mutex::new(None);
/// Bumpes ved hver accelerator-aendring: polleren nulstiller sin kant-tilstand
/// saa et hold paatvunget over en konfigurations-aendring ikke fyrer forkert.
static COMBO_VERSION: AtomicU64 = AtomicU64::new(0);
static SUSPENDED: AtomicBool = AtomicBool::new(false);
/// Talminal-vinduets HWND (sat ved setup). Fokus-gaten sammenligner det med
/// GetForegroundWindow pr. poll — 0 = ukendt = gate lukket (fail-closed).
static WINDOW_HWND: AtomicIsize = AtomicIsize::new(0);
static POLLER_STARTED: AtomicBool = AtomicBool::new(false);

/// Saet (eller udskift) den aktive accelerator. Parse-fejl rapporteres til
/// kalderen (frontenden viser den i HUD'et) — aldrig lydloes (redapting
/// review-fund #17/#47: "hotkeys virker bare ikke" uden fejl er usynligt).
pub fn set_accelerator(accel: &str) -> Result<(), String> {
    let combo = parse_accelerator(accel)?;
    let mut slot = COMBO.lock().map_err(|e| e.to_string())?;
    *slot = Some(combo);
    COMBO_VERSION.fetch_add(1, Ordering::Release);
    SUSPENDED.store(false, Ordering::Release);
    Ok(())
}

pub fn set_suspended(suspended: bool) {
    SUSPENDED.store(suspended, Ordering::Release);
    COMBO_VERSION.fetch_add(1, Ordering::Release);
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

#[cfg(windows)]
fn read_snapshot(trigger_vk: u16) -> PollSnapshot {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_CONTROL, VK_LWIN, VK_MENU, VK_RMENU, VK_RWIN, VK_SHIFT,
    };
    PollSnapshot {
        ctrl_or_meta: is_vk_down(VK_CONTROL) || is_vk_down(VK_LWIN) || is_vk_down(VK_RWIN),
        shift: is_vk_down(VK_SHIFT),
        alt: is_vk_down(VK_MENU),
        right_alt: is_vk_down(VK_RMENU),
        trigger_down: trigger_vk != 0 && is_vk_down(trigger_vk),
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
            let mut detector = ComboEdgeDetector::default();
            let mut vk_cache = TriggerVkCache::default();
            let mut seen_version = COMBO_VERSION.load(Ordering::Acquire);
            loop {
                thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
                let version = COMBO_VERSION.load(Ordering::Acquire);
                if version != seen_version {
                    seen_version = version;
                    detector.reset();
                    vk_cache = TriggerVkCache::default();
                }
                if SUSPENDED.load(Ordering::Acquire) {
                    continue;
                }
                let combo = match COMBO.lock() {
                    Ok(slot) => *slot,
                    Err(_) => continue,
                };
                let Some(combo) = combo else { continue };
                let (trigger_vk, warning) = vk_cache.vk_for(&combo.trigger, &Win32Resolver);
                if let Some(warning) = warning {
                    eprintln!("[wake-hotkey] {warning}");
                    publish_layout_warning(warning);
                }
                let snapshot = read_snapshot(trigger_vk);
                match detector.step(&combo, &snapshot) {
                    Step::Press => {
                        crate::perf_mark_background!(
                            "voice.ptt.press_sampled",
                            serde_json::json!({ "poll_interval_ms": POLL_INTERVAL_MS }),
                        );
                        let _ = app.emit(
                            WAKE_EVENT,
                            WakeHotkeyEvent {
                                combo: "ptt",
                                edge: "press",
                            },
                        );
                    }
                    Step::Release => {
                        crate::perf_mark_background!(
                            "voice.ptt.release_sampled",
                            serde_json::json!({ "poll_interval_ms": POLL_INTERVAL_MS }),
                        );
                        let _ = app.emit(
                            WAKE_EVENT,
                            WakeHotkeyEvent {
                                combo: "ptt",
                                edge: "release",
                            },
                        );
                    }
                    Step::SuppressedUnfocused => {
                        // Doede-tryk-diagnostik: rigtigt kombo-tryk, gate
                        // lukket. Ses denne samtidig med at brugeren kigger
                        // paa canvas, er fokus-maalingen forkert.
                        eprintln!(
                            "[wake-hotkey] kombo-tryk set, men canvas er ikke forgrundsvindue — kanten kasseret"
                        );
                    }
                    Step::SuppressedAltGr => {
                        eprintln!(
                            "[wake-hotkey] kombo-tryk set, men hoejre Alt (AltGr) er nede — kanten kasseret"
                        );
                    }
                    Step::SuppressedModifierDown => {
                        eprintln!(
                            "[wake-hotkey] bar binding: trigger nede, men en modifier diskvalificerede det eksakte match"
                        );
                    }
                    Step::SuppressedUnfocusedQuiet | Step::None => {}
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
        let raw = include_str!("../../hotkey-grammar.fixtures.json");
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
                Trigger::Mouse { .. } => panic!("{accel} burde vaere en tast"),
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
        assert_eq!(cache.vk_for(&trigger, &r).0, 0xBA);
        r.layout.set(2);
        r.map.borrow_mut().insert((0x27, false), 0xC0);
        assert_eq!(cache.vk_for(&trigger, &r).0, 0xC0);
    }

    #[test]
    fn vk_cache_beholder_sidste_gyldige_ved_nulretur_og_advarer_een_gang() {
        let r = FakeResolver {
            layout: std::cell::Cell::new(1),
            map: std::cell::RefCell::new([((0x56, false), 0xE2)].into_iter().collect()),
        };
        let trigger = parse_accelerator("Ctrl+IntlBackslash").unwrap().trigger;
        let mut cache = TriggerVkCache::default();
        assert_eq!(cache.vk_for(&trigger, &r).0, 0xE2);
        r.layout.set(2);
        r.map.borrow_mut().clear();
        let (vk, warning) = cache.vk_for(&trigger, &r);
        assert_eq!(vk, 0xE2);
        assert!(warning.expect("advarsel").contains("IntlBackslash"));
        assert_eq!(cache.vk_for(&trigger, &r).1, None);
    }

    #[test]
    fn vk_cache_springer_oploesning_over_for_mus() {
        let r = FakeResolver {
            layout: std::cell::Cell::new(1),
            map: std::cell::RefCell::new(std::collections::HashMap::new()),
        };
        let trigger = parse_accelerator("Mouse4").unwrap().trigger;
        let mut cache = TriggerVkCache::default();
        assert_eq!(cache.vk_for(&trigger, &r), (0x05, None));
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
            combo: "ptt",
            edge: "press",
        })
        .expect("serialize press payload");
        let release = serde_json::to_value(WakeHotkeyEvent {
            combo: "ptt",
            edge: "release",
        })
        .expect("serialize release payload");
        assert_eq!(press, serde_json::json!({"combo": "ptt", "edge": "press"}));
        assert_eq!(
            release,
            serde_json::json!({"combo": "ptt", "edge": "release"})
        );
    }

    #[test]
    fn set_accelerator_rejects_parse_errors() {
        assert!(set_accelerator("ikke-en-accelerator").is_err());
        assert!(set_accelerator("Ctrl+Shift+Space").is_ok());
    }

    #[test]
    fn suspension_er_ortogonal_til_set_accelerator() {
        let before = COMBO_VERSION.load(Ordering::Acquire);
        set_suspended(true);
        let during = COMBO_VERSION.load(Ordering::Acquire);
        assert!(during > before);
        assert!(is_suspended());
        set_accelerator("Alt+KeyQ").expect("set under suspension");
        assert!(!is_suspended());
        let after = COMBO_VERSION.load(Ordering::Acquire);
        assert!(after > during);
        let combo = COMBO.lock().expect("lock").expect("combo");
        assert_eq!(combo.trigger.code(), "KeyQ");
        set_accelerator(crate::workspace::DEFAULT_PTT_HOTKEY).expect("reset");
    }
}
