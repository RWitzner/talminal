// Settings-panel (Task 14). MOUNTES IKKE endnu — Task 17 mounter det i
// App.tsx. Panelet er selvbaerende: egne typer (types.ts er Task 9's lease),
// egne styles, al IO via invoke.
//
// Sikkerhedskontrakt: gemte noegler vises ALDRIG i klartekst. load_secret
// bruges kun til at afgoere sat/ikke-sat — vaerdien smides vaek med det samme
// og lander hverken i state, props eller DOM. Erstat-feltet er et password-
// input der toemmes efter gem.

import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  VoiceRoutes,
  WorkspaceResponse,
  WorkspaceSettings,
} from "./types";
import { HotkeyRecorder } from "./HotkeyRecorder";
import {
  DEFAULT_DICTATION_HOTKEY,
  DEFAULT_EXIT_TYPE_MODE_HOTKEY,
  DEFAULT_PTT_HOTKEY,
} from "./hotkeyDefaults";
import {
  DEFAULT_SETTINGS_CATEGORY,
  type SettingsCategory,
} from "./settingsCategories";
import { probeSttRoute } from "./voice/probe";
import { describeMicrophoneFailure } from "./voice/micError";
import { startBrowserCapture } from "./voice/ptt";
import {
  DEFAULT_WALLPAPER,
  WALLPAPERS,
  isWallpaperSlug,
  resolveWallpaperUrl,
  type WallpaperSlug,
} from "./wallpapers";

// Bindende noeglenavne — en slot pr. udbyder, uanset hvilken rolle
// udbyderen bruges til.
export const PROVIDER_KEY_OPENAI = "provider_key_openai";
export const PROVIDER_KEY_VERCEL = "provider_key_vercel";
export const PROVIDER_KEY_GOOGLE = "provider_key_google";
export const PROVIDER_KEY_OPENROUTER = "provider_key_openrouter";


async function saveSettingsPatch(
  patch: Partial<WorkspaceSettings>,
): Promise<void> {
  // set_settings skriver hele dokumentet. Hent derfor et friskt snapshot lige
  // før hver save, så én sektion ikke overskriver de andre med stale state.
  const workspace = await invoke<WorkspaceResponse>("get_workspace");
  const current = workspace.settings;
  await invoke("set_settings", {
    settings: {
      ptt_hotkey: patch.ptt_hotkey ?? current.ptt_hotkey ?? DEFAULT_PTT_HOTKEY,
      exit_type_mode_hotkey:
        patch.exit_type_mode_hotkey ??
        current.exit_type_mode_hotkey ??
        DEFAULT_EXIT_TYPE_MODE_HOTKEY,
      voice_engine: "pipeline",
      wallpaper:
        patch.wallpaper ??
        (isWallpaperSlug(current.wallpaper)
          ? current.wallpaper
          : DEFAULT_WALLPAPER),
      default_agent: patch.default_agent ?? current.default_agent ?? "claude",
      stt_provider: patch.stt_provider ?? current.stt_provider ?? "openai",
      routing_provider:
        patch.routing_provider ?? current.routing_provider ?? "vercel",
      dictation_hotkey:
        patch.dictation_hotkey ??
        current.dictation_hotkey ??
        DEFAULT_DICTATION_HOTKEY,
      // Alt-bindingerne SKAL med i hver eneste gem-vej — set_settings skriver
      // hele dokumentet, saa en udeladt alt-binding ville blive slettet af et
      // hvilket som helst gem i en anden sektion.
      //
      // `in` og IKKE `??`: her betyder `null` "ryd", og `??` falder igennem
      // paa netop `null`. Med `??` kunne en ryddet binding aldrig gemmes —
      // den ville hver gang falde tilbage til den vaerdi brugeren lige slettede.
      // Samme faelde som `dictation_submit` og `false` nedenfor.
      ptt_hotkey_alt:
        "ptt_hotkey_alt" in patch
          ? patch.ptt_hotkey_alt ?? null
          : current.ptt_hotkey_alt ?? null,
      dictation_hotkey_alt:
        "dictation_hotkey_alt" in patch
          ? patch.dictation_hotkey_alt ?? null
          : current.dictation_hotkey_alt ?? null,
      // `??` og ikke `||`: `false` er en gyldig vaerdi og maa ikke falde
      // igennem til current.
      dictation_submit:
        patch.dictation_submit ?? current.dictation_submit ?? false,
    },
  });
}

// ---------------------------------------------------------------------------
// Noeglerne (BYOK) — ÉN raekke pr. slot, datadrevet.
//
// Sikkerhedskontrakt: gemte noegler vises ALDRIG i klartekst. `load_secret`
// bruges kun til at afgoere sat/ikke-sat; vaerdien smides vaek med det samme og
// lander hverken i state, props eller DOM. Erstat-feltet er et password-input
// der toemmes efter gem.
//
// LISTEN SKAL DAEKKE ALLE SLOTS I RUSTS `providers.rs` (KEY_SLOT_*). Indtil
// 2026-07-29 havde UI'et kun OpenAI og Vercel, mens STT ogsaa kunne vaelges til
// OpenRouter og routeren til baade Google direkte og OpenRouter: man kunne
// vaelge en udbyder, hvis noegle der ikke fandtes et felt til, og eneste
// symptom var at ruten fejlede ved brug.
// ---------------------------------------------------------------------------

const SECRET_SLOTS = [
  {
    key: PROVIDER_KEY_OPENAI,
    label: "OpenAI",
    placeholder: "sk-…",
    // Det ENESTE slot der kan bære begge roller: vælges OpenAI som router
    // også, er hele stemme-vejen dækket af denne ene nøgle.
    help: "Stemme-genkendelsen — og routeren, hvis du vælger OpenAI som udbyder.",
  },
  {
    key: PROVIDER_KEY_VERCEL,
    label: "Vercel AI Gateway",
    placeholder: "vck_…",
    help: "Routeren, når udbyderen er Vercel AI Gateway.",
  },
  {
    key: PROVIDER_KEY_GOOGLE,
    label: "Google",
    placeholder: "AIza…",
    help: "Routeren, når udbyderen er Google direkte.",
  },
  {
    key: PROVIDER_KEY_OPENROUTER,
    label: "OpenRouter",
    placeholder: "sk-or-…",
    // Var "begge roller" indtil 2026-07-29, hvor OpenRouter-STT-ruten blev
    // slettet. Nøglen lever videre — som router-udbyder alene.
    help: "Routeren, når udbyderen er OpenRouter.",
  },
] as const;

type SecretSlot = (typeof SECRET_SLOTS)[number];

/**
 * Noeglesiden.
 *
 * Henter workspace-snapshottet ÉN gang for at kunne markere de to slots, det
 * aktuelle valg faktisk bruger. Markeringen kommer fra `voice_routes.stt
 * .key_slot` og `.routing.key_slot` — altsaa fra Rusts EGEN rutetabel — saa
 * den kan ikke drifte fra virkeligheden, som en spejlet TS-tabel ville kunne.
 */
export function SecretsSection() {
  const [kraevede, setKraevede] = useState<string[]>([]);

  useEffect(() => {
    let alive = true;
    invoke<WorkspaceResponse>("get_workspace")
      .then((workspace) => {
        const routes = workspace.voice_routes;
        if (!alive || routes == null) return;
        setKraevede([routes.stt.key_slot, routes.routing.key_slot]);
      })
      .catch(() => {
        // Markeringen er en hjaelp, ikke en funktion. Fejler opslaget, staar
        // raekkerne umarkerede i stedet for at vise en fejl, brugeren ikke kan
        // bruge til noget — noeglerne kan gemmes praecis som foer.
      });
    return () => {
      alive = false;
    };
  }, []);

  return (
    <>
      <div style={styles.sectionTitle}>
        BYOK — gemmes i Windows Credential Manager
      </div>
      {SECRET_SLOTS.map((slot) => (
        <SecretRow
          key={slot.key}
          slot={slot}
          required={kraevede.includes(slot.key)}
        />
      ))}
    </>
  );
}

function SecretRow({
  slot,
  required,
}: {
  slot: SecretSlot;
  /** Bruges af det aktuelle udbyder-valg — vises som maerkat, spaerrer intet. */
  required: boolean;
}) {
  // null = status ukendt (loader, eller load_secret fejlede).
  const [present, setPresent] = useState<boolean | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void invoke<boolean | null>("load_secret", { key: slot.key })
      .then((value) => {
        if (alive) setPresent(value === true);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });
    return () => {
      alive = false;
    };
  }, [slot.key]);

  // Hver mutation efterfulgt af et friskt opslag: knappernes disabled-tilstand
  // haenger paa `present`, og en optimistisk opdatering ville lyve, hvis
  // keyringen afviste skrivningen.
  const run = async (operation: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await operation();
      const value = await invoke<boolean | null>("load_secret", {
        key: slot.key,
      });
      setPresent(value === true);
    } catch (reason) {
      setError(String(reason));
    }
    setBusy(false);
  };

  const save = () =>
    run(async () => {
      if (draft.length === 0) return;
      await invoke("store_secret", { key: slot.key, value: draft });
      setDraft("");
    });
  const clear = () => run(() => invoke("delete_secret", { key: slot.key }));

  const status = present === null ? "ukendt" : present ? "sat" : "ikke sat";

  return (
    <div style={styles.row} data-secret-slot={slot.key}>
      <div style={styles.rowHeader}>
        <span style={styles.label}>{slot.label}</span>
        <span style={styles.rowHeaderRight}>
          {required && (
            <span data-secret-required style={styles.badgeRequired}>
              i brug
            </span>
          )}
          <SecretStatus
            known={present !== null}
            isSet={present === true}
            status={status}
          />
        </span>
      </div>
      <div style={styles.rowLead}>{slot.help}</div>
      <input
        style={styles.input}
        type="password"
        autoComplete="off"
        aria-label={`${slot.label}-nøgle`}
        placeholder={
          present === true
            ? `Indsæt ny nøgle for at erstatte (${slot.placeholder})…`
            : `Indsæt nøgle (${slot.placeholder})…`
        }
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") void save();
        }}
        disabled={busy}
      />
      <div style={styles.controls}>
        <button
          style={styles.button}
          onClick={() => void save()}
          disabled={busy || draft.length === 0}
        >
          Gem
        </button>
        <button
          style={styles.linkButton}
          onClick={() => void clear()}
          disabled={busy || present !== true}
          title="Fjern nøglen fra Windows Credential Manager"
        >
          Fjern
        </button>
      </div>
      {error !== null && <div style={styles.error}>{error}</div>}
    </div>
  );
}

/**
 * Statusprikken paa en noeglerraekke. Samme udtryk som rail'ens rad-prik, saa
 * "sat / ikke sat / ukendt" laeses paa farve OG ord — den gamle kant-badge
 * lignede en knap uden at vaere det.
 */
function SecretStatus({
  known,
  isSet,
  status,
}: {
  known: boolean;
  isSet: boolean;
  status: string;
}) {
  const dot: CSSProperties = !known
    ? { background: "#46505e" }
    : isSet
      ? { background: "#4dd6b7", boxShadow: "0 0 10px rgba(77, 214, 183, 0.5)" }
      : { background: "#e8b046", boxShadow: "0 0 10px rgba(232, 176, 70, 0.6)" };
  return (
    <span style={styles.status}>
      <span aria-hidden="true" style={{ ...styles.statusDot, ...dot }} />
      {status}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Hotkey-sektion: PTT + exit-type-mode (globalt settings.json via wiren;
// ikke keyring — B-light T4)
// ---------------------------------------------------------------------------

function doubleEffectNote(accel: string): string | null {
  if (/(^|\+)Mouse1$/u.test(accel) || /(^|\+)Mouse2$/u.test(accel)) {
    return "Hvert klik vækker også stemmen.";
  }
  if (/(^|\+)Mouse4$/u.test(accel) || /(^|\+)Mouse5$/u.test(accel)) {
    return "Browser-kort navigerer også frem/tilbage.";
  }
  if (/^Shift\+Escape$/iu.test(accel)) {
    return "Husets faste kombination for at forlade type-mode — den gør begge dele.";
  }
  if (/^Alt\+Space$/iu.test(accel) || /^Ctrl\+Shift\+Escape$/iu.test(accel)) {
    return "Windows gør også sit eget med den kombination.";
  }
  return null;
}

function HotkeySection({ onSaved }: { onSaved?: () => void | Promise<void> }) {
  const [ptt, setPtt] = useState(DEFAULT_PTT_HOTKEY);
  const [pttAlt, setPttAlt] = useState("");
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    invoke<WorkspaceResponse>("get_workspace")
      .then((ws) => {
        if (!alive || ws.settings == null) return;
        setPtt(ws.settings.ptt_hotkey);
        setPttAlt(ws.settings.ptt_hotkey_alt ?? "");
      })
      .catch((err) => {
        // Workspace ikke loaded (fx foer startup_load): behold defaults.
        if (alive) setError(String(err));
      });
    return () => {
      alive = false;
    };
  }, []);

  const save = async (accel: string) => {
    if (busy) return;
    const previous = ptt;
    setPtt(accel);
    setBusy(true);
    setSaved(false);
    setError(null);
    try {
      await saveSettingsPatch({
        ptt_hotkey: accel,
        voice_engine: "pipeline",
      });
      await onSaved?.();
      setSaved(true);
    } catch (err) {
      setPtt(previous);
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  // Ryd sender `null` og IKKE "": tom streng er en parse-fejl efter den delte
  // grammatik, saa den maa aldrig naa Rust-siden som en binding.
  const saveAlt = async (accel: string | null) => {
    if (busy) return;
    const previous = pttAlt;
    setPttAlt(accel ?? "");
    setBusy(true);
    setSaved(false);
    setError(null);
    try {
      await saveSettingsPatch({
        ptt_hotkey_alt: accel,
        voice_engine: "pipeline",
      });
      await onSaved?.();
      setSaved(true);
    } catch (err) {
      setPttAlt(previous);
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const note = doubleEffectNote(ptt);

  return (
    <div style={styles.row}>
      <div style={styles.rowHeader}>
        <span style={styles.label}>Stemme-aktivering</span>
        {saved && <span style={{ ...styles.badge, ...styles.badgeSet }}>gemt</span>}
      </div>
      <div style={styles.rowLead}>Hold nede for at tale.</div>
      {/* Nulstil og reglerne bor nu INDE i optageren: de hoerer til den
          handling, og som selvstaendig kant-knap plus tre linjer permanent
          graa tekst fyldte de mere end selve genvejen. */}
      <HotkeyRecorder
        value={ptt}
        onChange={save}
        onReset={() => void save(DEFAULT_PTT_HOTKEY)}
        disabled={busy}
      />
      {note !== null && <div style={styles.hint}>{note}</div>}
      <div style={styles.rowLead}>
        Ekstra genvej — begge virker samtidig. Til fx en controller-trigger, saa
        den samme funktion kan naas baade fra skrivebordet og fra et headset.
      </div>
      <HotkeyRecorder
        value={pttAlt}
        onChange={(accel) => saveAlt(accel)}
        onReset={pttAlt === "" ? undefined : () => void saveAlt(null)}
        resetLabel="Ryd"
        emptyLabel="Ingen ekstra"
        disabled={busy}
      />
      {error !== null && <div style={styles.error}>{error}</div>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Diktering: genvej + auto-send. Egen sektion frem for en raekke i
// HotkeySection, fordi kontakten hoerer sammen med genvejen og ikke med
// stemme-aktiveringen.
// ---------------------------------------------------------------------------

function DictationSection({ onSaved }: { onSaved?: () => void | Promise<void> }) {
  const [hotkey, setHotkey] = useState(DEFAULT_DICTATION_HOTKEY);
  const [hotkeyAlt, setHotkeyAlt] = useState("");
  const [submit, setSubmit] = useState(false);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    invoke<WorkspaceResponse>("get_workspace")
      .then((ws) => {
        if (!alive || ws.settings == null) return;
        setHotkey(ws.settings.dictation_hotkey ?? DEFAULT_DICTATION_HOTKEY);
        setHotkeyAlt(ws.settings.dictation_hotkey_alt ?? "");
        setSubmit(ws.settings.dictation_submit === true);
      })
      .catch((err) => {
        if (alive) setError(String(err));
      });
    return () => {
      alive = false;
    };
  }, []);

  const save = async (patch: {
    dictation_hotkey?: string;
    // `null` = ryd, `undefined` = urørt — derfor `in`-tjekket nedenfor og
    // ikke `!== undefined`: `null` skal kunne skrives igennem.
    dictation_hotkey_alt?: string | null;
    dictation_submit?: boolean;
  }) => {
    if (busy) return;
    const previous = { hotkey, hotkeyAlt, submit };
    if (patch.dictation_hotkey !== undefined) setHotkey(patch.dictation_hotkey);
    if ("dictation_hotkey_alt" in patch) {
      setHotkeyAlt(patch.dictation_hotkey_alt ?? "");
    }
    if (patch.dictation_submit !== undefined) setSubmit(patch.dictation_submit);
    setBusy(true);
    setSaved(false);
    setError(null);
    try {
      await saveSettingsPatch(patch);
      await onSaved?.();
      setSaved(true);
    } catch (err) {
      // Rust afviser bl.a. en genvej der deler tast med stemme-aktiveringen
      // (wake_hotkey::collides). Fejlteksten er brugervendt og vises ordret.
      setHotkey(previous.hotkey);
      setHotkeyAlt(previous.hotkeyAlt);
      setSubmit(previous.submit);
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const note = doubleEffectNote(hotkey);

  return (
    <div style={styles.row}>
      <div style={styles.rowHeader}>
        <span style={styles.label}>Diktering</span>
        {saved && <span style={{ ...styles.badge, ...styles.badgeSet }}>gemt</span>}
      </div>
      <div style={styles.rowLead}>
        Hold nede og tal — teksten skrives i det kort du står i, uden at gå
        gennem stemmekommandoerne.
      </div>
      <HotkeyRecorder
        value={hotkey}
        onChange={(accel) => save({ dictation_hotkey: accel })}
        onReset={() => void save({ dictation_hotkey: DEFAULT_DICTATION_HOTKEY })}
        disabled={busy}
      />
      {note !== null && <div style={styles.hint}>{note}</div>}
      <div style={styles.rowLead}>
        Ekstra genvej — begge virker samtidig.
      </div>
      <HotkeyRecorder
        value={hotkeyAlt}
        onChange={(accel) => save({ dictation_hotkey_alt: accel })}
        onReset={
          hotkeyAlt === ""
            ? undefined
            : () => void save({ dictation_hotkey_alt: null })
        }
        resetLabel="Ryd"
        emptyLabel="Ingen ekstra"
        disabled={busy}
      />
      <label style={styles.toggleRow}>
        <input
          type="checkbox"
          data-dictation-submit
          checked={submit}
          disabled={busy}
          onChange={(e) => void save({ dictation_submit: e.target.checked })}
        />
        <span>Send automatisk når du slipper</span>
      </label>
      {/* Konsekvensen skrives ud frem for at kalde det en "hurtig tilstand":
          uden den staar brugeren med en agent der koerer paa noget han ikke
          naaede at laese. */}
      <div style={styles.hint}>
        Slået fra skriver dikteringen kun teksten, så du kan rette den og selv
        trykke Enter. Slået til sendes den med det samme — også hvis den blev
        hørt forkert.
      </div>
      {error !== null && <div style={styles.error}>{error}</div>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Wallpaper-sektion: hvert klik persisterer straks og App re-henter derefter
// workspace-snapshot'et, så canvas-baggrunden skifter uden genstart.
// ---------------------------------------------------------------------------

function WallpaperSection({
  onSaved,
}: {
  onSaved?: () => void | Promise<void>;
}) {
  const [selected, setSelected] =
    useState<WallpaperSlug>(DEFAULT_WALLPAPER);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    invoke<WorkspaceResponse>("get_workspace")
      .then((workspace) => {
        if (!alive) return;
        const wallpaper = workspace.settings?.wallpaper;
        setSelected(
          isWallpaperSlug(wallpaper) ? wallpaper : DEFAULT_WALLPAPER,
        );
      })
      .catch((err) => {
        if (alive) setError(String(err));
      });
    return () => {
      alive = false;
    };
  }, []);

  const choose = async (wallpaper: WallpaperSlug) => {
    if (busy || wallpaper === selected) return;
    const previous = selected;
    setSelected(wallpaper);
    setBusy(true);
    setSaved(false);
    setError(null);
    try {
      await saveSettingsPatch({ wallpaper });
      await onSaved?.();
      setSaved(true);
    } catch (err) {
      setSelected(previous);
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={styles.row}>
      <div style={styles.rowHeader}>
        <span style={styles.label}>Canvas-baggrund</span>
        {saved && <span style={{ ...styles.badge, ...styles.badgeSet }}>gemt</span>}
      </div>
      <div style={styles.wallpaperGrid}>
        {WALLPAPERS.map((wallpaper) => {
          const isSelected = wallpaper.slug === selected;
          const wallpaperUrl = resolveWallpaperUrl(wallpaper.slug);
          return (
            <button
              key={wallpaper.slug}
              type="button"
              aria-label={`Vælg ${wallpaper.label}`}
              aria-pressed={isSelected}
              data-wallpaper-slug={wallpaper.slug}
              style={{
                ...styles.wallpaperButton,
                ...(isSelected ? styles.wallpaperButtonSelected : {}),
              }}
              disabled={busy}
              onClick={() => void choose(wallpaper.slug)}
            >
              {wallpaperUrl === null ? (
                <span
                  data-wallpaper-liquid-preview
                  style={styles.wallpaperLiquidPreview}
                />
              ) : (
                <img
                  src={wallpaperUrl}
                  alt=""
                  draggable={false}
                  style={styles.wallpaperImage}
                />
              )}
              <span style={styles.wallpaperLabel}>{wallpaper.label}</span>
            </button>
          );
        })}
      </div>
      <div style={styles.hint}>Skiftet anvendes straks uden genstart.</div>
      {error !== null && <div style={styles.error}>{error}</div>}
    </div>
  );
}

// Begge valg er OpenAI, saa labels er MODELNAVNE og ikke udbydernavne — det er
// modellen der er forskellen. (Routing-sektionen har udbydernavne, fordi dét
// valg peger paa forskellige leverandoerer.)
//
// Advarslen paa mini siger hvad vi VED (mindre model, hurtigere, billigere) og
// hvad vi ikke ved. Der staar med vilje ingen procent: mini er aldrig koert
// gennem voice-eval paa dansk.
const STT_CHOICES = [
  { slug: "openai", label: "gpt-4o-transcribe", warning: null },
  {
    slug: "openai-mini",
    label: "gpt-4o-mini-transcribe",
    warning:
      "Mindre model — hurtigere og ca. halv pris. Ikke målt på dansk; forvent flere fejlhøringer på kort-numre.",
  },
] as const;

const ROUTING_CHOICES = [
  { slug: "vercel", label: "Vercel AI Gateway", warning: null },
  { slug: "google", label: "Google direkte", warning: null },
  { slug: "openrouter", label: "OpenRouter", warning: null },
  // Den eneste router-udbyder der IKKE kræver en konto ud over OpenAI —
  // stemme-genkendelsen bruger allerede den nøgle. Noten er en oplysning om
  // afvejningen, ikke en fejl: præcisionen er målt til at være den samme
  // (Gate 1 41/41), men svartiden er omtrent den dobbelte.
  {
    slug: "openai",
    label: "OpenAI",
    warning: "Kun din OpenAI-nøgle. Samme præcision, ca. dobbelt svartid.",
  },
] as const;

export function ProviderSection({
  title,
  lead,
  choices,
  settingsKey,
  currentModelOf,
  probe,
  onSaved,
}: {
  title: string;
  /** Valgfri undertekst over valgene — fx hvilken noegle ruten kraever. */
  lead?: string;
  choices: ReadonlyArray<{
    slug: string;
    label: string;
    warning: string | null;
  }>;
  settingsKey: "stt_provider" | "routing_provider";
  currentModelOf: (routes: VoiceRoutes) => string;
  probe: () => Promise<string>;
  onSaved?: () => void | Promise<void>;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const [model, setModel] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [probeResult, setProbeResult] = useState<
    { ok: true; text: string } | { ok: false; reason: string } | null
  >(null);
  const [probing, setProbing] = useState(false);
  const modelOfRef = useRef(currentModelOf);
  modelOfRef.current = currentModelOf;

  useEffect(() => {
    let alive = true;
    invoke<WorkspaceResponse>("get_workspace")
      .then((workspace) => {
        if (!alive) return;
        setSelected(workspace.settings?.[settingsKey] ?? choices[0].slug);
        if (workspace.voice_routes) {
          setModel(modelOfRef.current(workspace.voice_routes));
        }
      })
      .catch((err) => {
        if (alive) setError(String(err));
      });
    return () => {
      alive = false;
    };
  }, [settingsKey, choices]);

  const choose = async (slug: string) => {
    if (busy || slug === selected) return;
    const previous = selected;
    setSelected(slug);
    setBusy(true);
    setError(null);
    // Probe-resultatet hoerte til den FORRIGE rute. Blev det staaende, ville
    // det groenne flueben staa under et valg der aldrig er testet — og
    // modeltagget ved siden af skifter samtidig til den nye model, saa raekken
    // ville paastaa at netop den model havde svaret.
    setProbeResult(null);
    try {
      await saveSettingsPatch({ [settingsKey]: slug });
      const workspace = await invoke<WorkspaceResponse>("get_workspace");
      if (workspace.voice_routes) setModel(currentModelOf(workspace.voice_routes));
      await onSaved?.();
    } catch (err) {
      setSelected(previous);
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const runProbe = async () => {
    setProbing(true);
    setProbeResult(null);
    try {
      setProbeResult({ ok: true, text: await probe() });
    } catch (err) {
      setProbeResult({ ok: false, reason: String(err) });
    } finally {
      setProbing(false);
    }
  };

  return (
    <div style={styles.row}>
      <div style={styles.rowHeader}>
        <span style={styles.label}>{title}</span>
      </div>
      {lead !== undefined && <div style={styles.rowLead}>{lead}</div>}
      {/* Hvert valg er et helt kort, ikke en radioknap med tekst ved siden af:
          hele fladen er klikbar, og advarslen staar INDE i det valg den
          gaelder — foer laa den som graa tekst under raekken og kunne lige saa
          godt hoere til den naeste. Selve <input type="radio"> beholdes (og
          bliver synlig): den baerer tastatur- og skaermlaeser-adfaerden, som en
          `role="radio"`-attrap skulle genopfinde. */}
      <div style={styles.choiceList}>
        {choices.map((choice) => {
          const valgt = selected === choice.slug;
          return (
            <label
              key={choice.slug}
              style={{
                ...styles.choice,
                ...(valgt ? styles.choiceSelected : null),
              }}
            >
              <input
                type="radio"
                name={settingsKey}
                value={choice.slug}
                data-provider-slug={choice.slug}
                checked={valgt}
                onChange={() => void choose(choice.slug)}
                disabled={busy}
                style={styles.choiceRadio}
              />
              <span style={styles.choiceText}>
                <span style={styles.choiceLabel}>{choice.label}</span>
                {choice.warning !== null && (
                  <span style={styles.choiceWarning}>{choice.warning}</span>
                )}
              </span>
            </label>
          );
        })}
      </div>

      <div style={styles.probeRow}>
        <button
          style={styles.button}
          onClick={() => void runProbe()}
          disabled={probing || busy}
        >
          {probing ? "Tester…" : "Test forbindelsen"}
        </button>
        {model !== null && <span style={styles.modelTag}>{model}</span>}
      </div>

      {probeResult !== null &&
        (probeResult.ok ? (
          <div style={styles.ok}>✓ {probeResult.text}</div>
        ) : (
          <div style={styles.error}>✗ {probeResult.reason}</div>
        ))}
      {error !== null && <div style={styles.error}>{error}</div>}
    </div>
  );
}

/**
 * Stemme-genkendelsen er et valg igen — men et ANDET valg end foer.
 *
 * Indtil 2026-07-29 valgte man UDBYDER (OpenAI eller OpenRouter). Da
 * OpenRouter-ruten blev slettet, stod sektionen tilbage som ren visning: et
 * radiovalg med én mulighed er ikke et valg. Fra 2026-08-04 vaelger man MODEL
 * inden for samme udbyder — samme realtime-session, samme noegle, samme
 * domaene-ordliste, kun vaegtklassen skifter. Derfor baerer valgene her ingen
 * advarsel om manglende kapabiliteter, som OpenRouter-valget gjorde: de to
 * ruter kan det samme, og forskellen er praecision mod fart og pris.
 *
 * Testknappen bliver staaende — den er det eneste sted man kan faa at vide om
 * noeglen og ruten virker, foer man staar med en stum mikrofon. Bemaerk hvad
 * den IKKE kan: bliver `prompt` tavst ignoreret af den mindre model, svarer
 * proben stadig paent, og det viser sig foerst som ringere genkendelse af
 * kort-numre i brug.
 */
export function VoiceProviderSection({
  onSaved,
}: {
  onSaved?: () => void | Promise<void>;
}) {
  return (
    <ProviderSection
      title="Stemme-genkendelse"
      lead="Via OpenAI. Kræver OpenAI-nøglen under Nøgler."
      choices={STT_CHOICES}
      settingsKey="stt_provider"
      currentModelOf={(routes) => routes.stt.model}
      // FRISKT snapshot pr. klik: proben skal teste den rute man lige har
      // valgt — ikke den der var valgt da sektionen blev monteret.
      probe={async () => {
        const workspace = await invoke<WorkspaceResponse>("get_workspace");
        return probeSttRoute(workspace.voice_routes);
      }}
      onSaved={onSaved}
    />
  );
}

/**
 * Mikrofon-tjek med en vej ud, når svaret er nej.
 *
 * Mikrofon-adgang kan blokeres tre steder — WebView2's egen prompt, Windows'
 * privatlivsindstilling, og hardwaren — og indtil nu opdagede man det først
 * midt i en ytring, som en engelsk DOMException i HUD'en uden nogen handling
 * knyttet til. Sektionen flytter opdagelsen frem til et roligt tidspunkt og
 * giver den knap der faktisk løser den hyppigste årsag.
 *
 * Testen går gennem `startBrowserCapture`, altså PRÆCIS den vej taleknappen
 * bruger. En lettere prøve (fx `enumerateDevices`) ville kunne lykkes mens den
 * rigtige vej stadig fejler — og så ville sektionen lyve.
 */
export function MicrophoneSection() {
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<
    { ok: true } | { ok: false; message: string; systemSettings: boolean } | null
  >(null);

  const runTest = async () => {
    setTesting(true);
    setResult(null);
    try {
      const capture = await startBrowserCapture(() => {});
      // Mikrofonen slippes straks igen — testen skal bevise adgang, ikke holde
      // enheden aabnet og blokere den rigtige optagelse bagefter.
      await capture.stop();
      setResult({ ok: true });
    } catch (error) {
      const failure = describeMicrophoneFailure(error);
      setResult({
        ok: false,
        message: failure.message,
        systemSettings: failure.systemSettings,
      });
    } finally {
      setTesting(false);
    }
  };

  return (
    <div style={styles.row} data-microphone-check>
      <div style={styles.rowHeader}>
        <span style={styles.label}>Mikrofon</span>
      </div>
      <div style={styles.rowLead}>
        Talminal svarer selv ja til mikrofon-anmodningen. Blokerer Windows
        adgangen, kan appen ikke omgå det — testen her siger hvad der mangler.
      </div>
      <div style={styles.probeRow}>
        <button
          style={styles.button}
          onClick={() => void runTest()}
          disabled={testing}
        >
          {testing ? "Tester…" : "Test mikrofonen"}
        </button>
        {result !== null && !result.ok && result.systemSettings && (
          <button
            style={styles.button}
            data-open-mic-settings
            onClick={() => {
              void invoke("open_microphone_settings").catch(() => {
                // Genvejen er en bekvemmelighed. Kan skallen ikke aabne siden,
                // staar vejvisningen stadig i fejlteksten ovenfor.
              });
            }}
          >
            Åbn Windows-indstillinger
          </button>
        )}
      </div>
      {result !== null &&
        (result.ok ? (
          <div style={styles.ok}>✓ Mikrofonen svarer.</div>
        ) : (
          <div style={styles.error}>✗ {result.message}</div>
        ))}
    </div>
  );
}

export function RoutingProviderSection({
  onSaved,
}: {
  onSaved?: () => void | Promise<void>;
}) {
  return (
    <ProviderSection
      title="Routing"
      choices={ROUTING_CHOICES}
      settingsKey="routing_provider"
      currentModelOf={(routes) => routes.routing.model}
      probe={() => invoke<string>("probe_router_route")}
      onSaved={onSaved}
    />
  );
}

export function ResetProvidersButton({
  onSaved,
}: {
  onSaved?: () => void | Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const reset = async () => {
    setBusy(true);
    setError(null);
    try {
      await saveSettingsPatch({
        stt_provider: "openai",
        routing_provider: "vercel",
      });
      await onSaved?.();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div style={styles.row}>
      <button style={styles.button} onClick={() => void reset()} disabled={busy}>
        Nulstil til anbefalet
      </button>
      {/* Knappen bor under Routing, men roerer BEGGE roller — og
          stemme-genkendelsen er en anden kategori, saa den aendring sker uden
          for skaermen. Derfor staar det skrevet. */}
      {/* Uden modelnavne i teksten: `providers.rs` er den eneste kilde til
          dem, og en streng her ville vaere endnu en kopi at holde synkron. */}
      <div style={styles.hint}>
        Sætter både stemme-genkendelsen og routingen tilbage til de anbefalede
        ruter.
      </div>
      {error !== null && <div style={styles.error}>{error}</div>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Agent-sektion: default_agent er choke-pointet i create_card (Task 5) —
// nye kort uden eksplicit agent-valg (voice ELLER manuel spawn-dialog)
// arver denne vaerdi. Samme save-/frisk-snapshot-moenster som wallpaper.
// ---------------------------------------------------------------------------

const AGENT_OPTIONS = [
  { slug: "claude", label: "Claude Code" },
  { slug: "codex", label: "Codex" },
] as const;

function AgentSection({
  onSaved,
}: {
  onSaved?: () => void | Promise<void>;
}) {
  const [agent, setAgent] = useState("claude");
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    invoke<WorkspaceResponse>("get_workspace")
      .then((workspace) => {
        if (!alive) return;
        setAgent(workspace.settings?.default_agent ?? "claude");
      })
      .catch((err) => {
        if (alive) setError(String(err));
      });
    return () => {
      alive = false;
    };
  }, []);

  const choose = async (value: string) => {
    if (busy || value === agent) return;
    const previous = agent;
    setAgent(value);
    setBusy(true);
    setSaved(false);
    setError(null);
    try {
      await saveSettingsPatch({ default_agent: value });
      await onSaved?.();
      setSaved(true);
    } catch (err) {
      setAgent(previous);
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={styles.row}>
      <div style={styles.rowHeader}>
        <span style={styles.label}>Standard-agent</span>
        {saved && <span style={{ ...styles.badge, ...styles.badgeSet }}>gemt</span>}
      </div>
      <div style={styles.controls}>
        <select
          data-default-agent-select
          style={styles.input}
          value={agent}
          disabled={busy}
          onChange={(event) => void choose(event.target.value)}
        >
          {AGENT_OPTIONS.map(({ slug, label }) => (
            <option key={slug} value={slug}>
              {label}
            </option>
          ))}
        </select>
      </div>
      <div style={styles.hint}>
        Codex skal være installeret (npm) og logget ind via `codex login`.
        Gælder nye kort. Voice-agentvalg kræver pipeline-engine.
      </div>
      {error !== null && <div style={styles.error}>{error}</div>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Panelet
// ---------------------------------------------------------------------------

/**
 * Kategorierne bor i `settingsCategories.ts`, ikke her — se modulets egen
 * begrundelse. De re-eksporteres, saa kaldere der allerede taler med
 * `./Settings` ikke skal kende to moduler; `SettingsWindow` importerer
 * bevidst fra det lille modul for at holde lazy-chunken lazy.
 *
 * "Tastatur" findes ikke laengere som egen kategori: den havde praecis én
 * indstilling (stemme-aktiveringen), og den handler om stemmen — ikke om
 * tastaturet. Den bor derfor under `voice` sammen med STT-udbyderen.
 */
export {
  SETTINGS_CATEGORIES,
  DEFAULT_SETTINGS_CATEGORY,
  type SettingsCategory,
} from "./settingsCategories";

/**
 * ÉN kategori ad gangen — vinduet (SettingsWindow) ejer valget og tegner
 * navigationen. Komponenten er bevidst stadig ét lazy-chunk: kategorierne
 * deler `saveSettingsPatch`, og hver underkomponent henter sit eget snapshot
 * ved mount, saa et skift mellem to kategorier er en montering og ikke en
 * ny netvaerksrunde for hele panelet.
 */
export function Settings({
  category = DEFAULT_SETTINGS_CATEGORY,
  onSaved,
}: {
  category?: SettingsCategory;
  onSaved?: () => void | Promise<void>;
}) {
  return (
    <section style={styles.panel} data-settings-category={category}>
      {category === "voice" && (
        <>
          <HotkeySection onSaved={onSaved} />
          <DictationSection onSaved={onSaved} />
          <MicrophoneSection />
          {/* onSaved er ikke kosmetik: den forer valget videre til App.tsx'
              refresh, som opdaterer `voiceRoutesRef` — uden den taler den
              KOERENDE session videre til den gamle model. */}
          <VoiceProviderSection onSaved={onSaved} />
        </>
      )}

      {category === "keys" && <SecretsSection />}

      {category === "routing" && (
        <>
          <RoutingProviderSection onSaved={onSaved} />
          <ResetProvidersButton onSaved={onSaved} />
        </>
      )}

      {category === "agents" && <AgentSection onSaved={onSaved} />}

      {category === "appearance" && <WallpaperSection onSaved={onSaved} />}
    </section>
  );
}

export default Settings;

// Husets glas-sprog (samme toner som WorkspaceRail og SettingsWindow). Panelet
// havde sit eget: fladt #161a20, graa #374151-kanter og Tailwind-graat tekst —
// det saa ud som et udviklerpanel skruet paa en designet app.
const styles: Record<string, CSSProperties> = {
  // Vinduet baerer kant, baggrund og titel. Panelet er derfor en ren
  // indholdsbeholder — med sin egen ramme blev hver kategori en kasse-i-kasse.
  panel: {
    display: "flex",
    flexDirection: "column",
    gap: 10,
    color: "#b9c8d9",
    fontFamily: '"Segoe UI", system-ui, sans-serif',
    fontSize: 13,
  },
  sectionTitle: {
    margin: 0,
    color: "#718297",
    fontSize: 10,
    fontWeight: 700,
    letterSpacing: "0.14em",
    textTransform: "uppercase",
  },
  row: {
    display: "flex",
    flexDirection: "column",
    gap: 8,
    border: "1px solid rgba(209, 232, 251, 0.1)",
    borderRadius: 10,
    padding: "13px 14px",
    background: "rgba(8, 18, 32, 0.34)",
  },
  rowHeader: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
  },
  rowHeaderRight: { display: "flex", alignItems: "center", gap: 8 },
  // "i brug" — den noegle dit aktuelle udbyder-valg faktisk raekker efter.
  // Maerkatet spaerrer intet: man skal kunne gemme en noegle FOER man skifter
  // udbyder, ellers er der ingen vej ind i den nye udbyder.
  badgeRequired: {
    borderRadius: 999,
    padding: "1px 8px",
    background: "rgba(122, 182, 232, 0.16)",
    color: "#9fd0ff",
    fontSize: 10,
    fontWeight: 600,
    letterSpacing: "0.04em",
  },
  label: { color: "#edf5fc", fontSize: 13, fontWeight: 600 },
  // Én linje der siger hvad kontrollen ER TIL, foer man moeder den.
  rowLead: { marginTop: -2, color: "#8ea0b5", fontSize: 12 },
  badge: {
    padding: "1px 9px",
    borderRadius: 999,
    border: "1px solid rgba(222, 241, 255, 0.18)",
    color: "#8ea0b5",
    fontSize: 11,
  },
  badgeSet: { borderColor: "rgba(77, 214, 183, 0.5)", color: "#4dd6b7" },
  status: {
    display: "flex",
    alignItems: "center",
    gap: 7,
    color: "#8ea0b5",
    fontSize: 11,
  },
  statusDot: { width: 8, height: 8, flex: "0 0 auto", borderRadius: 3 },
  controls: { display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" },
  input: {
    boxSizing: "border-box",
    width: "100%",
    border: "1px solid rgba(222, 241, 255, 0.16)",
    borderRadius: 8,
    padding: "8px 10px",
    background: "rgba(6, 14, 26, 0.6)",
    color: "#edf5fc",
    font: "inherit",
    fontSize: 13,
  },
  button: {
    border: "1px solid rgba(222, 241, 255, 0.18)",
    borderRadius: 8,
    padding: "7px 13px",
    background:
      "radial-gradient(90% 120% at 0% 0%, rgba(113, 180, 225, 0.16), transparent 58%), linear-gradient(145deg, rgba(24, 46, 70, 0.46), rgba(2, 9, 20, 0.4))",
    color: "#c8d9ea",
    font: "inherit",
    fontSize: 12,
    cursor: "pointer",
  },
  // Stille handling — bruges hvor knappen er sekundaer eller destruktiv, saa
  // "Fjern" ikke vejer visuelt lige saa meget som "Gem".
  linkButton: {
    border: 0,
    borderRadius: 5,
    padding: 0,
    background: "transparent",
    color: "#8ea0b5",
    font: "inherit",
    fontSize: 12,
    textDecoration: "underline",
    textUnderlineOffset: 2,
    cursor: "pointer",
  },
  choiceList: { display: "flex", flexDirection: "column", gap: 6 },
  choice: {
    display: "flex",
    alignItems: "flex-start",
    gap: 10,
    border: "1px solid rgba(209, 232, 251, 0.1)",
    borderRadius: 9,
    padding: "10px 11px",
    background: "rgba(6, 14, 26, 0.3)",
    cursor: "pointer",
    transition: "border-color 140ms ease, background 140ms ease",
  },
  choiceSelected: {
    borderColor: "rgba(160, 209, 255, 0.4)",
    background: "rgba(122, 182, 232, 0.13)",
  },
  choiceRadio: { marginTop: 2, flex: "0 0 auto", accentColor: "#7ab6e8" },
  choiceText: { display: "flex", flexDirection: "column", gap: 3, minWidth: 0 },
  choiceLabel: { color: "#edf5fc", fontSize: 13 },
  choiceWarning: { color: "#8ea0b5", fontSize: 11, lineHeight: 1.5 },
  probeRow: { display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" },
  modelTag: {
    borderRadius: 6,
    padding: "3px 8px",
    background: "rgba(6, 14, 26, 0.6)",
    color: "#8ea0b5",
    fontFamily: '"Cascadia Mono", monospace',
    fontSize: 11,
  },
  wallpaperGrid: {
    display: "grid",
    gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
    gap: 9,
  },
  wallpaperButton: {
    position: "relative",
    minWidth: 0,
    padding: 0,
    aspectRatio: "16 / 10",
    overflow: "hidden",
    border: "1px solid rgba(222, 241, 255, 0.16)",
    borderRadius: 9,
    background: "rgba(6, 14, 26, 0.6)",
    color: "#f3f4f6",
    cursor: "pointer",
  },
  wallpaperButtonSelected: {
    borderColor: "rgba(160, 209, 255, 0.7)",
    boxShadow: "0 0 0 2px rgba(103, 183, 255, 0.28)",
  },
  wallpaperImage: {
    display: "block",
    width: "100%",
    height: "100%",
    objectFit: "cover",
  },
  wallpaperLiquidPreview: {
    display: "block",
    width: "100%",
    height: "100%",
    background: [
      "radial-gradient(ellipse 90% 72% at 22% 8%, rgba(116, 195, 255, 0.3), transparent 64%)",
      "radial-gradient(ellipse 76% 82% at 94% 78%, rgba(92, 80, 190, 0.24), transparent 68%)",
      "linear-gradient(132deg, rgba(8, 22, 44, 0.72), rgba(3, 8, 18, 0.42))",
    ].join(", "),
  },
  wallpaperLabel: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
    padding: "14px 7px 6px",
    background: "linear-gradient(transparent, rgba(5, 8, 14, 0.9))",
    color: "#f3f4f6",
    fontSize: 11,
    fontWeight: 600,
    textAlign: "left",
  },
  hint: { fontSize: 11, color: "#718297", lineHeight: 1.5 },
  toggleRow: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    fontSize: 12,
    color: "#c8d6e6",
    cursor: "pointer",
  },
  ok: { fontSize: 11, color: "#4dd6b7" },
  error: { fontSize: 11, color: "#ff9b93" },
};
