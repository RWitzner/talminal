/**
 * Delte testhjælpere for frontenden.
 *
 * `deferred` stod byte-identisk i seks testfiler, og de to flush-former i ni.
 * De er samlet her — men de er IKKE slået sammen til én, for de tre
 * ventemåder i suiten betyder forskellige ting, og en fælles `flush()` ville
 * skjule hvilken man bad om:
 *
 *   - `flushTimers()`   — giver plads til en MAKRO-task (setTimeout 0).
 *                          Bruges af voice-suiten, hvor transporten selv
 *                          poster over en timer.
 *   - `flushMicrotasks()` — dræner en kæde af await-trin uden at give
 *                          timeren en tur. Bruges af Card-suiten, hvor
 *                          effekterne kun kæder promises.
 *
 * Den tredje form — `await act(async () => {})` — bor stadig lokalt hos den
 * ene test der bruger den: den hører til React-renderen, ikke til
 * ventemekanikken.
 */

/** En promise med sine egne resolve/reject i hånden — så en test kan holde et
 *  kald hængende og afgøre præcis hvornår det lander. */
export function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Én tur gennem makrotask-køen. */
export async function flushTimers(): Promise<void> {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

/** Dræner mikrotask-køen. Seks runder er nok til de effekt-kæder Card-suiten
 *  venter på — tallet er arvet fra de fire kopier det erstatter. */
export async function flushMicrotasks(): Promise<void> {
  for (let index = 0; index < 6; index += 1) await Promise.resolve();
}
