import { performance } from "node:perf_hooks";

const CHUNKS = 250_000;
const SAMPLES = 7;
const payload = {
  name: "card-source",
  data_b64: "A".repeat(5_464),
};

function median(values) {
  return [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
}

function measure(cardCount) {
  const discardHandlers = Array.from(
    { length: Math.max(0, cardCount - 1) },
    (_, index) => {
      const name = `card-discard-${index}`;
      return (event) => {
        if (event.name !== name) return false;
        throw new Error("benchmark payload unexpectedly matched");
      };
    },
  );

  // Warm V8 before the timed samples.
  for (let chunk = 0; chunk < 20_000; chunk += 1) {
    for (const handler of discardHandlers) handler(payload);
  }

  const samples = [];
  for (let sample = 0; sample < SAMPLES; sample += 1) {
    const started = performance.now();
    for (let chunk = 0; chunk < CHUNKS; chunk += 1) {
      for (const handler of discardHandlers) handler(payload);
    }
    samples.push(performance.now() - started);
  }

  const discardedCalls = CHUNKS * discardHandlers.length;
  const totalMs = discardedCalls === 0 ? 0 : median(samples);
  return {
    cards: cardCount,
    chunks: CHUNKS,
    discarded_calls: discardedCalls,
    discarded_handler_ms: Number(totalMs.toFixed(3)),
    wasted_us_per_chunk: Number(((totalMs * 1_000) / CHUNKS).toFixed(4)),
    ns_per_discarded_call:
      discardedCalls === 0
        ? 0
        : Number(((totalMs * 1_000_000) / discardedCalls).toFixed(2)),
  };
}

console.log(
  JSON.stringify(
    {
      runtime: process.version,
      method:
        "median of 7 warmed samples; handler body matches Card.tsx name-mismatch fast path",
      results: [1, 5, 10].map(measure),
    },
    null,
    2,
  ),
);
