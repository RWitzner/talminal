// Chat-kortet (agent-til-agent, spec §6). Kortet er ejerens vindue ind i en
// traad mellem to agenter: det VISER udvekslingen, og det lader ejeren skyde en
// besked ind eller stoppe samarbejdet. Der er ingen PTY og ingen webview — kun
// traadens egen laesemodel (chat_thread_read) plus `chat-thread-updated` som
// primaer opdateringskilde. Uden eventet ser ejeren kun sine EGNE beskeder, og
// hele §5.4's samtykke-argument — at udvekslingen ER synlig — falder.
//
// Praesentationen hviler paa to greb, og begge er afledt af traadens egne
// regler frem for af chat-konventioner:
//
//  1. RYGGEN. Hver agent faar en fast accent efter foerste optraeden, tegnet
//     som en 2 px stribe langs sine beskeder. Ejeren er IKKE paa den skala —
//     hans beskeder er rykket ind med en stiplet stribe, fordi han ikke er en
//     tur-tager: hans beskeder koster ikke hop. Systembeskeder bryder ryggen
//     helt og staar som regibemaerkninger mellem haarlinjer. Farven er altid
//     et SUPPLEMENT: afsenderen staar ogsaa som tekst (spec §6).
//  2. LUNTEN. Hop-taelleren er traadens levetid — ved loftet lukker den. Den
//     tegnes derfor som en braendende maaler frem for et tal i en krog.

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ChatCardInfo } from "./types";
import { CARD_HEADER, CARD_NUMBER_BADGE, CARD_SHELL } from "./cardChrome";
import { cardLabel } from "./cardLabel";
import {
  assignAgentSlots,
  formatClock,
  hopTone,
  splitFencedSegments,
} from "./chatText";

interface ThreadMessage {
  seq: number;
  from_card: string;
  from_kind: "agent" | "human" | "system";
  intent: string;
  text: string;
  ts_ms: number;
}

interface ThreadView {
  state: "open" | "awaiting" | "closed";
  purpose: string;
  hops_used: number;
  hops_left: number;
  messages: ThreadMessage[];
}

export interface ChatCardProps {
  card: ChatCardInfo;
  fullscreen: boolean;
  onToggleFullscreen(): void;
}

/** Traadens medlemmer er praecis to, saa 0 og 1 er de eneste pladser der
 *  bruges i praksis. Den tredje er en bevidst KEDELIG stalfarve: sker det
 *  alligevel, skal den ikke kunne forveksles med de to rigtige. */
const AGENT_ACCENTS = ["#74c0ff", "#c3a0ff", "#9fb8d4"];
/** Ejeren staar uden for accent-skalaen — han er en anden slags stemme. */
const OWNER_ACCENT = "#dfe9f4";

/** Kun fallback indtil traaden selv har vist sit loft (og for en traad der
 *  allerede stod ved loftet da kortet blev monteret). Spejler `MAX_HOPS`. */
const DEFAULT_HOP_CAP = 20;

/** Tekstlig afsender-label. Farve alene er ikke en label (spec §6).
 *  Selve "Kort N"-oversaettelsen kommer fra `cardLabel` — den regel bor ét
 *  sted. En lokal `replace(/^card-/, "")` her gav et ANDET svar for
 *  toml-seedede kort: "Kort master (agent)" i stedet for "master (agent)". */
function senderLabel(m: ThreadMessage): string {
  if (m.from_kind === "system") return "System";
  if (m.from_kind === "human") return "Dig (menneske)";
  return `${cardLabel(m.from_card)} (agent)`;
}

/** `delegation` aabner en forpligtelse og `answer` lukker den — de to er
 *  parret og faar vaegt. `sparring`/`status` er default-stoej og staar stille.
 *  Ordene er wire-ordene med vilje: runbooken henviser til dem ordret. */
const LOUD_INTENTS: Record<string, CSSProperties> = {
  // Bevidst NEUTRAL-lys frem for blaa: blaa er en agent-accent, og en blaa chip
  // paa en blaa afsender ville se ud som om den hoerte til ryggen.
  delegation: {
    border: "1px solid rgba(207, 232, 255, 0.3)",
    background: "rgba(207, 232, 255, 0.1)",
    color: "#dce8f4",
  },
  answer: {
    border: "1px solid rgba(94, 214, 192, 0.28)",
    background: "rgba(24, 86, 74, 0.34)",
    color: "#96dbc9",
  },
};

export function ChatCard({ card, fullscreen, onToggleFullscreen }: ChatCardProps) {
  const [view, setView] = useState<ThreadView | null>(null);
  const [draft, setDraft] = useState("");
  const logRef = useRef<HTMLOListElement | null>(null);
  const atBottomRef = useRef(true);
  const lastSeqRef = useRef(0);
  const generationRef = useRef(0);
  const appliedGenerationRef = useRef(0);
  const mountedRef = useRef(false);
  const threadRef = useRef(card.thread_id);
  // Opdateres under render, saa et allerede afsendt svar fra den forrige
  // traad kan kasseres selv i vinduet foer effect-cleanup koerer.
  threadRef.current = card.thread_id;

  const refresh = useCallback(async (replace = false) => {
    const thread = card.thread_id;
    const generation = ++generationRef.current;
    const fromSeq = replace ? 0 : lastSeqRef.current;
    const next = await invoke<ThreadView>("chat_thread_read", {
      thread,
      fromSeq,
    });
    if (!mountedRef.current || threadRef.current !== thread || next == null) return;

    const newest = generation > appliedGenerationRef.current;
    if (newest) {
      appliedGenerationRef.current = generation;
      lastSeqRef.current = Math.max(
        lastSeqRef.current,
        ...next.messages.map((message) => message.seq),
      );
    }

    setView((current) => {
      const messages = mergeMessages(
        replace && newest ? [] : current?.messages ?? [],
        next.messages,
      );
      // Et sent svar fra samme traad maa gerne bidrage beskeder (seq-dedup er
      // nok), men aldrig rulle state/hop-headeren tilbage.
      if (!newest && current !== null) return { ...current, messages };
      if (!newest) return current;
      return { ...next, messages };
    });
  }, [card.thread_id]);

  // Nulstiller laesetilstanden naar traaden skifter. Selve foerste laesning bor
  // i lytter-effekten nedenfor — se begrundelsen dér. Derfor er `refresh` stadig
  // dependency selv om den ikke kaldes her: den er traad-identiteten, og de to
  // effekter SKAL koere i lockstep, ellers nulstilles generations-vagten ikke
  // foran den laesning der lige er startet.
  useEffect(() => {
    mountedRef.current = true;
    lastSeqRef.current = 0;
    appliedGenerationRef.current = generationRef.current;
    setView(null);
    return () => {
      mountedRef.current = false;
      // Invalider alle svar der allerede er paa vej, ogsaa ved unmount.
      generationRef.current += 1;
    };
  }, [refresh]);

  // Eventet er kortets PRIMAERE opdateringskilde, og netop derfor ligger foerste
  // laesning HER — efter registreringen — og ikke i mount-effekten ovenfor.
  // `listen()` er en asynkron round-trip til Rust, og emitteren er en
  // LEVEL-baseret aendringsdetektor, ikke en retry: `heartbeat::beat`
  // sammenligner beskedantallet med `tracker.seen` og gemmer det nye tal med det
  // samme, saa den samme tilstand emittes aldrig igen. Et event der falder i
  // registreringsvinduet er dermed tabt for altid: havde vi laest FOER vinduet,
  // stod kortet med det foraeldede svar til naeste gang traaden tilfaeldigvis
  // voksede. Ejeren saa kun sine EGNE replikker, og hele §5.4's
  // samtykke-argument — at udvekslingen ER synlig — faldt.
  //
  // Den sene start er ufarlig: mount-effekten har lige sat
  // `appliedGenerationRef` til den aktuelle generation, saa denne laesning er
  // nyere end alt hvad der maatte vaere i luften fra den forrige traad, og et
  // event der naar at fyre foerst starter blot en LAVERE generation.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<{ thread: string }>("chat-thread-updated", (event) => {
      if (event.payload.thread === card.thread_id) void refresh();
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
        void refresh(true);
      })
      // Foerste laesning haenger nu paa at registreringen lykkes. Afvises
      // `listen()`, sker der derfor INTET: kortet staar tomt uden en eneste
      // kvittering, hvor det foer naaede at vise sit mount-snapshot. Chat-kortet
      // har ingen synlig fejlkanal (ingen `loadError`-pendant), og en ny flade
      // er ikke fundets aerinde — konsollen er det bedste sted fejlen kan naa
      // hen. Samme form som Card.tsx' card-exit-lytter.
      .catch((err) =>
        console.error(
          `chat-thread-updated listener(${card.thread_id}) fejlede:`,
          err,
        ),
      );
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [card.thread_id, refresh]);

  // Hold bunden hvis brugeren i forvejen ER i bund; ellers lad den staa, saa en
  // indkommende besked ikke river ham vaek fra det han laeste.
  useEffect(() => {
    const log = logRef.current;
    if (log && atBottomRef.current) log.scrollTop = log.scrollHeight;
  }, [view]);

  const messages = useMemo(() => view?.messages ?? [], [view]);
  const agentSlots = useMemo(
    () =>
      assignAgentSlots(
        messages.filter((m) => m.from_kind === "agent").map((m) => m.from_card),
      ),
    [messages],
  );

  const state = view?.state ?? "open";
  const closed = state === "closed";
  // Loftet laeses af traaden selv frem for at vaere haardkodet: `hops_left` er
  // `MAX.saturating_sub(used)`, saa brugt + tilbage ER loftet — men KUN saa
  // laenge der er noget tilbage.
  //
  // Ved loftet taeller `post()` hop op UBETINGET, ogsaa for det ene `answer`
  // der har lov at passere. Saa bliver used 21 mens left er 0, og udledningen
  // ville flytte loftet med og vise "21 / 21", som om intet saerligt skete.
  // Det er stik modsat hvad aflaesningen skal vise: at taelleren staar ÉT over
  // loftet ER signaturen paa at answer-undtagelsen fyrede. Derfor huskes det
  // sidst observerede loft og bruges naar left er i nul.
  const capRef = useRef(DEFAULT_HOP_CAP);
  const cap =
    view && view.hops_left > 0 ? view.hops_used + view.hops_left : capRef.current;
  useEffect(() => {
    if (view && view.hops_left > 0) capRef.current = view.hops_used + view.hops_left;
  }, [view]);
  const used = view?.hops_used ?? 0;
  const tone = hopTone(used, cap);

  async function send(): Promise<void> {
    const text = draft.trim();
    if (!text) return;
    try {
      await invoke("chat_thread_post", { thread: card.thread_id, text });
      setDraft(""); // Kladden ryddes KUN ved succes.
      await refresh();
    } catch {
      /* kladden bevares saa arbejdet ikke tabes */
    }
  }

  return (
    <div style={styles.root} data-chat-card={card.thread_id} data-chat-state={state}>
      <style>{chatCss}</style>
      <div style={styles.header}>
        <span style={styles.badge}>{card.number}</span>
        <span style={styles.purpose} data-chat-purpose title={card.purpose}>
          {card.purpose}
        </span>
        {/* Traad-id'et er ikke pynt: runbookens §4 beder ejeren aabne
            `threads\<tN>.jsonl` naar noget er roedt, og det er her han
            aflaeser HVILKEN fil. */}
        <span style={styles.threadId} title={`Arkiv: ${card.thread_id}.jsonl`}>
          {card.thread_id}
        </span>
        {state === "awaiting" && (
          <span
            style={styles.awaitingChip}
            data-chat-awaiting
            title="En delegering er udestaaende — nye delegeringer afvises indtil der svares"
          >
            <span style={styles.awaitingDot} data-chat-awaiting-dot aria-hidden="true" />
            afventer
          </span>
        )}
        {closed && (
          <span style={styles.closedChip} data-chat-closed>
            lukket
          </span>
        )}
        <span
          style={{ ...styles.hops, ...hopFill(used, cap, tone) }}
          data-chat-hops
          data-chat-hop-tone={tone}
          title={`${used} af ${cap} agent-beskeder brugt. Ved loftet lukker traaden. Dine egne og systemets beskeder taeller ikke med.`}
        >
          {`${used} / ${cap}`}
        </span>
        {!closed && (
          <button
            type="button"
            data-chat-stop
            title="Stop samarbejdet — traaden lukkes for begge agenter"
            style={styles.stopButton}
            onClick={() => {
              void invoke("chat_thread_stop", { thread: card.thread_id }).then(() => refresh());
            }}
          >
            <span style={styles.stopLabel}>Stop samarbejde</span>
          </button>
        )}
        <button
          type="button"
          data-chat-fullscreen-action
          onClick={onToggleFullscreen}
          title={fullscreen ? "Afslut fuldskærm" : "Fuldskærm"}
          aria-label={fullscreen ? "Afslut fuldskærm" : "Fuldskærm"}
          style={styles.iconButton}
        >
          {fullscreen ? "⤡" : "⤢"}
        </button>
      </div>
      <ol
        role="log"
        aria-label={`Traad ${card.thread_id}`}
        ref={logRef}
        data-chat-log
        style={styles.log}
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {view !== null && messages.length === 0 && (
          <li style={styles.empty} data-chat-empty>
            {closed ? "Traaden lukkede uden beskeder." : "Venter på den første besked."}
          </li>
        )}
        {messages.map((m) => {
          if (m.from_kind === "system") {
            return (
              <li key={m.seq} data-message-kind="system" style={styles.systemRow}>
                <span style={styles.systemLabel} data-chat-sender>
                  System
                </span>
                <span style={styles.systemText}>{m.text}</span>
              </li>
            );
          }
          const owner = m.from_kind === "human";
          const accent = owner
            ? OWNER_ACCENT
            : AGENT_ACCENTS[(agentSlots.get(m.from_card) ?? 0) % AGENT_ACCENTS.length];
          return (
            <li
              key={m.seq}
              data-message-kind={m.from_kind}
              style={{
                ...styles.message,
                ...(owner ? styles.ownerMessage : null),
                borderLeftColor: accent,
              }}
            >
              <div style={styles.meta}>
                <span style={{ ...styles.sender, color: accent }} data-chat-sender>
                  {senderLabel(m)}
                </span>
                {!owner && (
                  <span style={{ ...styles.intent, ...(LOUD_INTENTS[m.intent] ?? null) }}>
                    {m.intent}
                  </span>
                )}
                <span style={styles.clock}>{formatClock(m.ts_ms)}</span>
              </div>
              {/* Teksten deles kun op — den muteres aldrig. Kodeblokke faar
                  hele bredden og deres egen vandrette rulning, saa runbookens
                  §2.8 kan aflaeses direkte i kortet frem for i arkivet. */}
              {splitFencedSegments(m.text).map((segment, index) =>
                segment.kind === "code" ? (
                  <pre key={index} style={styles.code} data-chat-code>
                    {segment.body}
                  </pre>
                ) : (
                  <p key={index} style={styles.text}>
                    {segment.body}
                  </p>
                ),
              )}
            </li>
          );
        })}
      </ol>
      {/* En lukket traad har intet skrivefelt OG ingen fodnote: aarsagen staar
          allerede som traadens sidste systembesked, og loggen ruller til bunds,
          saa den ER det sidste ejeren ser. En strimmel her gentog den ordret. */}
      {!closed && (
        <div style={styles.composer}>
          <textarea
            data-chat-input
            aria-label="Skriv i traaden"
            placeholder="Skriv med i traaden…"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              // IME: en igangvaerende komposition maa ikke submitte.
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                void send();
              }
            }}
            style={styles.input}
          />
          <span style={styles.composerHint}>Enter sender · Shift+Enter ny linje</span>
        </div>
      )}
    </div>
  );
}

function mergeMessages(
  current: ThreadMessage[],
  incoming: ThreadMessage[],
): ThreadMessage[] {
  if (incoming.length === 0) return current;
  const bySeq = new Map(current.map((message) => [message.seq, message]));
  for (const message of incoming) bySeq.set(message.seq, message);
  return Array.from(bySeq.values()).sort((a, b) => a.seq - b.seq);
}

/** Lunten: sporet fyldes fra venstre efterhaanden som hop braendes. Ét
 *  element, ingen ekstra DOM — fyldet ER pillens baggrund. */
function hopFill(used: number, cap: number, tone: ReturnType<typeof hopTone>): CSSProperties {
  const percent = cap > 0 ? Math.min(100, Math.max(0, (used / cap) * 100)) : 0;
  const fill = {
    calm: "rgba(116, 192, 255, 0.34)",
    warn: "rgba(224, 166, 74, 0.44)",
    spent: "rgba(226, 96, 96, 0.5)",
  }[tone];
  const text = { calm: "#9eafc1", warn: "#e8c48d", spent: "#f0b9b9" }[tone];
  const edge = {
    calm: "rgba(187, 211, 233, 0.14)",
    warn: "rgba(224, 166, 74, 0.34)",
    spent: "rgba(226, 96, 96, 0.4)",
  }[tone];
  return {
    color: text,
    borderColor: edge,
    background: `linear-gradient(90deg, ${fill} 0 ${percent}%, rgba(112, 137, 163, 0.12) ${percent}% 100%)`,
  };
}

// Tilstande der ikke kan udtrykkes som inline-style: hover, focus-visible,
// scrollbar og reduceret bevaegelse. Samme greb som CanvasSurface/App.
const chatCss = `
  /* Canvas-roden saetter user-select: none, saa et traek paa fladen ikke
     markerer. Chat-kortet er et DOKUMENT, ikke en flade — det melder sig ud
     igen. Hele kortet, ikke kun loggen: traad-id og formaalstekst i headeren
     er praecis de to strenge runbookens §4 beder ejeren aflaese og bruge. */
  [data-chat-card] { user-select: text; }
  /* Knapperne holdes udenfor: et fejlklik maa markere ingenting. */
  [data-chat-card] button { user-select: none; }
  [data-chat-card] button {
    transition: background-color 140ms ease, border-color 140ms ease, color 140ms ease;
  }
  [data-chat-card] button:hover {
    background: rgba(129, 158, 189, 0.24);
    border-color: rgba(187, 211, 233, 0.3);
    color: #e6eef8;
  }
  [data-chat-card] [data-chat-stop]:hover {
    background: rgba(152, 44, 44, 0.6);
    border-color: rgba(255, 150, 150, 0.42);
    color: #ffd9d9;
  }
  [data-chat-card] button:focus-visible,
  [data-chat-card] textarea:focus-visible {
    outline: 2px solid rgba(116, 192, 255, 0.8);
    outline-offset: 1px;
  }
  [data-chat-card] textarea:focus {
    border-color: rgba(116, 192, 255, 0.45);
    background: rgba(2, 6, 13, 0.92);
  }
  [data-chat-card] [data-chat-log]::-webkit-scrollbar { width: 9px; }
  [data-chat-card] [data-chat-log]::-webkit-scrollbar-track { background: transparent; }
  [data-chat-card] [data-chat-log]::-webkit-scrollbar-thumb {
    background: rgba(129, 158, 189, 0.22);
    border: 3px solid transparent;
    background-clip: content-box;
    border-radius: 9px;
  }
  [data-chat-card] [data-chat-log]::-webkit-scrollbar-thumb:hover {
    background: rgba(150, 180, 212, 0.4);
    background-clip: content-box;
  }
  @keyframes talminal-chat-breathe {
    0%, 100% { opacity: 0.35; }
    50% { opacity: 1; }
  }
  [data-chat-card] [data-chat-awaiting-dot] {
    animation: talminal-chat-breathe 2.4s ease-in-out infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    [data-chat-card] [data-chat-awaiting-dot] { animation: none; opacity: 1; }
  }
`;

const styles: Record<string, CSSProperties> = {
  root: {
    ...CARD_SHELL,
    height: "100%",
    minHeight: 0,
    // Chat-kortet er ren tekst — farve og grundstoerrelse saettes paa roden og
    // arves ned, hvor de to andre korttyper saetter dem pr. element.
    color: "#cfe0f2",
    fontSize: 12,
  },
  header: { ...CARD_HEADER, overflow: "hidden" },
  badge: {
    ...CARD_NUMBER_BADGE,
    flex: "0 0 auto",
  },
  purpose: {
    flex: "1 1 auto",
    minWidth: 0,
    color: "#e2ebf4",
    fontWeight: 600,
    letterSpacing: "-0.005em",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  threadId: {
    flex: "0 0 auto",
    color: "#6f8296",
    fontFamily: '"Cascadia Mono", monospace',
    fontSize: 9,
  },
  awaitingChip: {
    flex: "0 0 auto",
    display: "flex",
    alignItems: "center",
    gap: 5,
    padding: "1px 8px 1px 6px",
    border: "1px solid rgba(116, 192, 255, 0.45)",
    borderRadius: 999,
    background: "rgba(45, 92, 138, 0.5)",
    color: "#cfe7ff",
    fontSize: 9,
    fontWeight: 600,
    letterSpacing: "0.02em",
    whiteSpace: "nowrap",
  },
  awaitingDot: {
    width: 5,
    height: 5,
    borderRadius: "50%",
    background: "#74c0ff",
    boxShadow: "0 0 6px rgba(116, 192, 255, 0.8)",
  },
  closedChip: {
    flex: "0 0 auto",
    padding: "1px 7px",
    border: "1px solid rgba(187, 211, 233, 0.16)",
    borderRadius: 999,
    background: "rgba(112, 137, 163, 0.14)",
    color: "#8fa3b8",
    fontSize: 9,
    whiteSpace: "nowrap",
  },
  hops: {
    flex: "0 0 auto",
    minWidth: 44,
    boxSizing: "border-box",
    padding: "2px 7px",
    border: "1px solid rgba(187, 211, 233, 0.14)",
    borderRadius: 999,
    textAlign: "center",
    fontVariantNumeric: "tabular-nums",
    fontSize: 9,
    fontWeight: 600,
    letterSpacing: "0.01em",
    whiteSpace: "nowrap",
  },
  stopButton: {
    // `0 1 auto` + minWidth 0: naar tilen bliver smal, ellipsizes etiketten i
    // stedet for at skubbe fuldskaermsknappen ud over headerens kant.
    flex: "0 1 auto",
    minWidth: 0,
    overflow: "hidden",
    padding: "2px 8px",
    // Daempet i hvile, fuldt roed paa hover: handlingen er uigenkaldelig, men
    // knappen staar fremme hele tiden. Vaegten hoerer til naar man raekker ud
    // efter den — ikke som en konstant alarm i headeren.
    border: "1px solid rgba(226, 120, 120, 0.26)",
    borderRadius: 6,
    background: "rgba(96, 34, 34, 0.26)",
    color: "#d79f9f",
    cursor: "pointer",
    fontFamily: "inherit",
    fontSize: 10,
  },
  stopLabel: {
    display: "block",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  iconButton: {
    flex: "0 0 auto",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    width: 22,
    height: 22,
    padding: 0,
    border: "1px solid rgba(187, 211, 233, 0.14)",
    borderRadius: 6,
    background: "rgba(112, 137, 163, 0.12)",
    color: "#9eafc1",
    cursor: "pointer",
    fontSize: 12,
    lineHeight: 1,
  },
  log: {
    flex: 1,
    minHeight: 0,
    overflowY: "auto",
    listStyle: "none",
    margin: 0,
    padding: "10px 10px 12px",
    display: "flex",
    flexDirection: "column",
    gap: 9,
  },
  empty: {
    margin: "auto 0",
    padding: "18px 4px",
    color: "#5c6d80",
    fontSize: 11,
    textAlign: "center",
  },
  message: {
    display: "flex",
    flexDirection: "column",
    gap: 3,
    padding: "5px 9px 6px",
    // Rene longhands (ikke `borderLeft`-shorthand): accenten og ejerens
    // stiplede ryg saettes som longhands ovenpaa, og React advarer naar de to
    // former blandes for samme vaerdi.
    borderLeftWidth: 2,
    borderLeftStyle: "solid",
    borderLeftColor: "transparent",
    borderRadius: "0 7px 7px 0",
    background: "rgba(112, 137, 163, 0.07)",
  },
  ownerMessage: {
    // Ejeren er ikke en tur-tager: hans beskeder koster ikke hop, saa de staar
    // rykket ind med stiplet ryg — uden for turoekonomien, bogstaveligt.
    marginLeft: 18,
    borderLeftStyle: "dashed",
    background: "rgba(146, 173, 200, 0.11)",
  },
  meta: {
    display: "flex",
    alignItems: "center",
    gap: 7,
    minWidth: 0,
  },
  sender: {
    flex: "0 1 auto",
    minWidth: 0,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    fontWeight: 600,
    fontSize: 10,
    letterSpacing: "0.015em",
  },
  intent: {
    flex: "0 0 auto",
    padding: "0 5px",
    borderRadius: 4,
    border: "1px solid transparent",
    color: "#6f8296",
    fontSize: 9,
    letterSpacing: "0.02em",
    whiteSpace: "nowrap",
  },
  clock: {
    marginLeft: "auto",
    flex: "0 0 auto",
    color: "#546679",
    fontVariantNumeric: "tabular-nums",
    fontSize: 9,
  },
  text: {
    margin: 0,
    whiteSpace: "pre-wrap",
    overflowWrap: "anywhere",
    color: "#cfe0f2",
    fontSize: 12,
    lineHeight: 1.55,
  },
  code: {
    margin: "3px 0 1px",
    padding: "7px 9px",
    overflowX: "auto",
    borderRadius: 6,
    border: "1px solid rgba(151, 184, 218, 0.14)",
    background: "rgba(2, 6, 13, 0.75)",
    color: "#d8e2ef",
    fontFamily: '"Cascadia Mono", Consolas, monospace',
    fontSize: 11,
    lineHeight: 1.5,
    whiteSpace: "pre",
    tabSize: 4,
  },
  systemRow: {
    // Regibemaerkning: traaden fortaeller om sig selv. Ingen ryg, ingen boks —
    // kun haarlinjer, saa den ikke kan forveksles med en deltager der taler.
    display: "flex",
    alignItems: "baseline",
    justifyContent: "center",
    gap: 8,
    margin: "2px 0",
    padding: "7px 4px",
    borderTop: "1px solid rgba(207, 232, 255, 0.09)",
    borderBottom: "1px solid rgba(207, 232, 255, 0.09)",
    textAlign: "center",
  },
  systemLabel: {
    flex: "0 0 auto",
    color: "#617488",
    fontSize: 9,
    fontWeight: 700,
    letterSpacing: "0.09em",
    textTransform: "uppercase",
  },
  systemText: {
    color: "#a3b6ca",
    fontSize: 11,
    lineHeight: 1.5,
  },
  composer: {
    flex: "0 0 auto",
    display: "flex",
    flexDirection: "column",
    gap: 4,
    padding: "8px 10px 9px",
    borderTop: "1px solid rgba(207, 232, 255, 0.06)",
  },
  input: {
    resize: "none",
    minHeight: 46,
    maxHeight: 132,
    padding: "7px 9px",
    border: "1px solid rgba(151, 184, 218, 0.22)",
    borderRadius: 7,
    background: "rgba(3, 8, 16, 0.7)",
    color: "#cfe0f2",
    // Ejeren skriver dansk prosa til to agenter, ikke shell-kommandoer.
    fontFamily: "inherit",
    fontSize: 12,
    lineHeight: 1.5,
    outline: "none",
  },
  composerHint: {
    color: "#546679",
    fontSize: 9,
    letterSpacing: "0.01em",
  },
};
