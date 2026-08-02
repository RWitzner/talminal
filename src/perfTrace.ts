import { invoke } from "@tauri-apps/api/core";

import type { CardInfo } from "./types";

// Keep this as a direct literal comparison. Vite substitutes the value at
// build time, allowing Rollup to erase every PERF_ENABLED branch (including
// event strings and argument construction) from a normal production bundle.
export const PERF_ENABLED =
  typeof import.meta.env !== "undefined" &&
  import.meta.env.VITE_TALMINAL_PERF === "1";

type PerfFields = Record<string, unknown>;

type FrontendMark = {
  event: string;
  tsEpochMs: number;
  fields: PerfFields;
};

export type PerfTrace = {
  id: string;
  kind: "create" | "close" | "voice";
  clientStartedMs: number;
  marks: FrontendMark[];
  sent: number;
  flushChain: Promise<void>;
  flags: Set<string>;
};

let traceSequence = 0;
let activeVoiceTrace: PerfTrace | null = null;
const pendingCreates = new Map<string, PerfTrace>();
const pendingCloses = new Map<string, PerfTrace>();

function epochNow(): number {
  return performance.timeOrigin + performance.now();
}

function nextTraceId(kind: PerfTrace["kind"]): string {
  traceSequence += 1;
  const suffix =
    typeof crypto?.randomUUID === "function"
      ? crypto.randomUUID()
      : String(Date.now()) + "-" + String(traceSequence);
  return kind + "-" + suffix;
}

export function startPerfTrace(
  kind: PerfTrace["kind"],
  fields: PerfFields = {},
  clientStartedMs = epochNow(),
): PerfTrace | null {
  if (!PERF_ENABLED) return null;
  const trace: PerfTrace = {
    id: nextTraceId(kind),
    kind,
    clientStartedMs,
    marks: [],
    sent: 0,
    flushChain: Promise.resolve(),
    flags: new Set(),
  };
  markPerf(trace, kind + ".frontend.start", fields);
  return trace;
}

export function markPerf(
  trace: PerfTrace | null | undefined,
  event: string,
  fields: PerfFields = {},
): void {
  if (!trace) return;
  trace.marks.push({ event, tsEpochMs: epochNow(), fields });
}

export function markPerfOnce(
  trace: PerfTrace | null | undefined,
  flag: string,
  event: string,
  fields: PerfFields = {},
): boolean {
  if (!trace || trace.flags.has(flag)) return false;
  trace.flags.add(flag);
  markPerf(trace, event, fields);
  return true;
}

/**
 * Run after the browser has had at least one paint opportunity. This is still
 * a compositor proxy (not a photon measurement), but unlike a single rAF it
 * cannot run in the same pre-paint callback phase as the observed commit.
 */
export function afterPaintOpportunity(callback: () => void): void {
  requestAnimationFrame(() => requestAnimationFrame(callback));
}

export function perfInvokeArgs(trace: PerfTrace | null | undefined): {
  perfTrace?: string;
  perfStartedMs?: number;
} {
  return trace
    ? { perfTrace: trace.id, perfStartedMs: trace.clientStartedMs }
    : {};
}

export function flushPerfTrace(
  trace: PerfTrace | null | undefined,
  finalBatch = false,
): Promise<void> {
  if (!trace) return Promise.resolve();
  const start = trace.sent;
  const end = trace.marks.length;
  const marks = trace.marks.slice(start, end);
  trace.sent = end;
  if (marks.length === 0 && !finalBatch) return trace.flushChain;
  trace.flushChain = trace.flushChain
    .then(() =>
      invoke("perf_trace_frontend", {
        traceId: trace.id,
        kind: trace.kind,
        clientStartedMs: trace.clientStartedMs,
        marks,
        finalBatch,
      }),
    )
    .then(
      () => undefined,
      () => undefined,
    );
  return trace.flushChain;
}

export function registerPendingCreate(name: string, trace: PerfTrace | null): void {
  if (trace) pendingCreates.set(name, trace);
}

export function createTraceFor(name: string): PerfTrace | null {
  if (!PERF_ENABLED) return null;
  return pendingCreates.get(name) ?? null;
}

export function completePendingCreate(
  name: string,
  trace: PerfTrace | null | undefined,
): void {
  if (trace && pendingCreates.get(name) === trace) pendingCreates.delete(name);
}

export function registerPendingClose(name: string, trace: PerfTrace | null): void {
  if (trace) pendingCloses.set(name, trace);
}

export function closeTraceFor(name: string): PerfTrace | null {
  if (!PERF_ENABLED) return null;
  return pendingCloses.get(name) ?? null;
}

export function completePendingClose(
  name: string,
  trace: PerfTrace | null | undefined,
): boolean {
  if (!trace || pendingCloses.get(name) !== trace) return false;
  pendingCloses.delete(name);
  return ![...pendingCloses.values()].some((candidate) => candidate === trace);
}

export function pendingCreateEntries(): Array<[string, PerfTrace]> {
  return [...pendingCreates.entries()];
}

export function pendingCloseEntries(): Array<[string, PerfTrace]> {
  return [...pendingCloses.entries()];
}

export function beginVoiceTrace(origin: string): PerfTrace | null {
  const trace = startPerfTrace("voice", { origin });
  activeVoiceTrace = trace;
  return trace;
}

export function getActiveVoiceTrace(): PerfTrace | null {
  return activeVoiceTrace;
}

export function finishVoiceTrace(trace: PerfTrace | null): void {
  if (activeVoiceTrace === trace) activeVoiceTrace = null;
  void flushPerfTrace(trace, true);
}

type PerfHarnessDeps = {
  refresh(trace?: PerfTrace | null): Promise<void>;
};

type PerfHarness = {
  createCards(
    cwd: string,
    count: number,
    concurrent?: boolean,
  ): Promise<CardInfo[]>;
  closeCardsSeparate(names: string[]): Promise<unknown[]>;
  closeCardsBatch(names: string[]): Promise<unknown>;
  openBrowser(url?: string | null, openedBy?: string | null): Promise<CardInfo>;
  listCards(): Promise<CardInfo[]>;
};

declare global {
  interface Window {
    __TALMINAL_PERF__?: PerfHarness;
  }
}

export function installPerfHarness({ refresh }: PerfHarnessDeps): () => void {
  if (!PERF_ENABLED) return () => undefined;

  const invokeCreate = async (cwd: string, trace: PerfTrace): Promise<CardInfo> => {
    markPerf(trace, "create.frontend.invoke.begin");
    try {
      const info = await invoke<CardInfo>("create_card", {
        cwd,
        command: null,
        ...perfInvokeArgs(trace),
      });
      markPerf(trace, "create.frontend.invoke.resolved", {
        card: info.name,
        running: info.running,
      });
      registerPendingCreate(info.name, trace);
      return info;
    } catch (error) {
      markPerf(trace, "create.frontend.invoke.rejected", {
        error: String(error),
      });
      void flushPerfTrace(trace, true);
      throw error;
    }
  };

  const refreshTrace = async (trace: PerfTrace): Promise<void> => {
    markPerf(trace, trace.kind + ".frontend.refresh.requested");
    await refresh(trace);
    markPerf(trace, trace.kind + ".frontend.refresh.resolved");
    void flushPerfTrace(trace);
  };

  const harness: PerfHarness = {
    async createCards(cwd, count, concurrent = false) {
      if (!Number.isInteger(count) || count < 1 || count > 10) {
        throw new Error("count must be an integer from 1 to 10");
      }
      let infos: CardInfo[];
      if (concurrent) {
        const traces = Array.from({ length: count }, (_, index) => {
          const trace = startPerfTrace("create", {
            entry: "perf_harness_concurrent",
            index,
            count,
          });
          if (!trace) throw new Error("performance tracing is disabled");
          return trace;
        });
        infos = await Promise.all(
          traces.map(async (trace) => {
            const info = await invokeCreate(cwd, trace);
            await refreshTrace(trace);
            return info;
          }),
        );
      } else {
        infos = [];
        for (let index = 0; index < count; index += 1) {
          const trace = startPerfTrace("create", {
            entry: "perf_harness_sequential",
            index,
            count,
          });
          if (!trace) throw new Error("performance tracing is disabled");
          const info = await invokeCreate(cwd, trace);
          infos.push(info);
          await refreshTrace(trace);
        }
      }
      return infos;
    },

    async closeCardsSeparate(names) {
      const jobs = names.map(async (name, index) => {
        const trace = startPerfTrace("close", {
          entry: "perf_harness_separate",
          index,
          count: names.length,
          names,
        });
        if (!trace) throw new Error("performance tracing is disabled");
        registerPendingClose(name, trace);
        markPerf(trace, "close.frontend.invoke.begin", { name });
        try {
          const result = await invoke("close_card", {
            name,
            ...perfInvokeArgs(trace),
          });
          markPerf(trace, "close.frontend.invoke.resolved", { name });
          await refreshTrace(trace);
          return result;
        } catch (error) {
          markPerf(trace, "close.frontend.invoke.rejected", {
            name,
            error: String(error),
          });
          void flushPerfTrace(trace, true);
          throw error;
        }
      });
      const results = await Promise.all(jobs);
      return results;
    },

    async closeCardsBatch(names) {
      const trace = startPerfTrace("close", {
        entry: "perf_harness_batch",
        count: names.length,
        names,
      });
      if (!trace) throw new Error("performance tracing is disabled");
      for (const name of names) registerPendingClose(name, trace);
      markPerf(trace, "close.frontend.invoke.begin", { names });
      try {
        const result = await invoke("close_cards", {
          names,
          ...perfInvokeArgs(trace),
        });
        markPerf(trace, "close.frontend.invoke.resolved", { names });
        await refreshTrace(trace);
        return result;
      } catch (error) {
        markPerf(trace, "close.frontend.invoke.rejected", {
          names,
          error: String(error),
        });
        void flushPerfTrace(trace, true);
        throw error;
      }
    },

    async openBrowser(url = null, openedBy = null) {
      const info = await invoke<CardInfo>("create_browser_card", {
        url,
        openedBy,
      });
      await refresh();
      return info;
    },

    listCards() {
      return invoke<CardInfo[]>("list_cards");
    },
  };

  window.__TALMINAL_PERF__ = harness;
  return () => {
    if (window.__TALMINAL_PERF__ === harness) {
      delete window.__TALMINAL_PERF__;
    }
  };
}
