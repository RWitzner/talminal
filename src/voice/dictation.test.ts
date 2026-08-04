import { afterEach, describe, expect, it, vi } from "vitest";
import { createDictationSession, type DictationUiState } from "./dictation";
import type { SttClient } from "./stt";
import { deferred, flushTimers } from "../testHelpers";

function fakeStt(final: Promise<string> | string = "kør testene igen"): SttClient {
  return {
    start: vi.fn(async () => undefined),
    pushAudio: vi.fn(),
    onPartial: vi.fn(),
    stop: vi.fn(async () => final),
    abort: vi.fn(),
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

function makeHarness(
  options: {
    stts?: SttClient[];
    insert?: (text: string) => Promise<void>;
  } = {},
) {
  const stts = options.stts ?? [fakeStt()];
  let sttIndex = 0;
  const states: DictationUiState[] = [];
  const transcripts: string[] = [];
  const errors: Error[] = [];
  const inserted: string[] = [];
  let emptyTurns = 0;
  const insert = vi.fn(
    options.insert ??
      (async (text: string) => {
        inserted.push(text);
      }),
  );
  const warm = vi.fn();
  const startCapture = vi.fn(async () => ({
    stop: vi.fn(async () => undefined),
  }));
  const session = createDictationSession({
    stt: () => stts[Math.min(sttIndex++, stts.length - 1)],
    insert,
    startCapture,
    onState: (state) => states.push(state),
    onTranscript: (text) => transcripts.push(text),
    onEmpty: () => {
      emptyTurns += 1;
    },
    onError: (error) => errors.push(error),
    warm,
  });
  return {
    session,
    stts,
    states,
    transcripts,
    errors,
    inserted,
    insert,
    warm,
    startCapture,
    emptyTurns: () => emptyTurns,
  };
}

async function runTurn(h: ReturnType<typeof makeHarness>) {
  h.session.press();
  await flushTimers();
  h.session.release();
  await flushTimers();
  await flushTimers();
}

describe("createDictationSession", () => {
  it("indsaetter transskriptet og lander idle", async () => {
    const h = makeHarness();
    await runTurn(h);
    expect(h.inserted).toEqual(["kør testene igen"]);
    expect(h.states).toEqual(["listening", "finalizing", "inserting", "idle"]);
    expect(h.session.state()).toBe("idle");
  });

  it("varmer forbindelserne ved tryk", async () => {
    const h = makeHarness();
    h.session.press();
    expect(h.warm).toHaveBeenCalledTimes(1);
    h.session.cancel();
  });

  it("trimmer teksten foer den indsaettes", async () => {
    const h = makeHarness({ stts: [fakeStt("   ryd op i imports   ")] });
    await runTurn(h);
    expect(h.inserted).toEqual(["ryd op i imports"]);
  });

  it("indsaetter INTET naar der ikke blev hoert noget", async () => {
    // En tom tur er ikke en fejl — mikrofonen kan have hoert stilhed. Men
    // den maa heller ikke skrive en tom streng i agentens composer.
    const h = makeHarness({ stts: [fakeStt("   ")] });
    await runTurn(h);
    expect(h.inserted).toEqual([]);
    expect(h.insert).not.toHaveBeenCalled();
    expect(h.emptyTurns()).toBe(1);
    expect(h.errors).toEqual([]);
    expect(h.session.state()).toBe("idle");
  });

  it("rapporterer en fejl fra indsaettelsen", async () => {
    const h = makeHarness({
      insert: async () => {
        throw new Error("write_pty fejlede");
      },
    });
    await runTurn(h);
    expect(h.errors.map((e) => e.message)).toEqual(["write_pty fejlede"]);
    expect(h.session.state()).toBe("idle");
  });

  it("kasserer den foerste tur naar der trykkes igen midt i den", async () => {
    // "Jeg vil sige noget andet": den gamle tur maa ikke naa composeren,
    // ellers kappes to ture om det samme tekstfelt.
    const first = deferred<string>();
    const h = makeHarness({
      stts: [fakeStt(first.promise), fakeStt("den nye sætning")],
    });
    h.session.press();
    await flushTimers();
    h.session.release();
    await flushTimers();

    h.session.press();
    await flushTimers();
    first.resolve("den gamle sætning");
    await flushTimers();
    h.session.release();
    await flushTimers();
    await flushTimers();

    expect(h.inserted).toEqual(["den nye sætning"]);
  });

  it("indsaetter ikke efter cancel", async () => {
    const final = deferred<string>();
    const h = makeHarness({ stts: [fakeStt(final.promise)] });
    h.session.press();
    await flushTimers();
    h.session.release();
    await flushTimers();
    h.session.cancel();
    final.resolve("for sent");
    await flushTimers();
    await flushTimers();
    expect(h.inserted).toEqual([]);
    expect(h.session.state()).toBe("idle");
  });

  it("ignorerer et slip der ikke hoerer til et hold", async () => {
    const h = makeHarness();
    h.session.release();
    await flushTimers();
    expect(h.states).toEqual([]);
    expect(h.insert).not.toHaveBeenCalled();
  });
});
