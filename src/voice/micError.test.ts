import { describe, expect, it } from "vitest";
import { describeMicrophoneFailure, MicrophoneError } from "./micError";

// Teksterne HER er alt brugeren faar at vide naar mikrofonen siger nej —
// App.tsx' onError viser fejlens message ordret i HUD'en. Testene laaser derfor
// ikke bare at der KOMMER en besked, men at den navngiver stedet man skal hen.

/** DOMException-agtig fejl; happy-dom har ikke alle konstruktoerer. */
function domError(name: string): Error {
  const error = new Error(`${name}: fejlede`);
  error.name = name;
  return error;
}

describe("describeMicrophoneFailure", () => {
  it("peger paa Windows-indstillingen ved NotAllowedError", () => {
    const failure = describeMicrophoneFailure(domError("NotAllowedError"));
    expect(failure.systemSettings).toBe(true);
    expect(failure.canRetry).toBe(true);
    expect(failure.message).toContain("Privatliv");
    expect(failure.message).toContain("Mikrofon");
  });

  it("behandler det aeldre PermissionDeniedError som samme sag", () => {
    const gammel = describeMicrophoneFailure(domError("PermissionDeniedError"));
    const ny = describeMicrophoneFailure(domError("NotAllowedError"));
    expect(gammel).toEqual(ny);
  });

  it("skelner manglende enhed fra manglende tilladelse", () => {
    const failure = describeMicrophoneFailure(domError("NotFoundError"));
    expect(failure.message).toContain("Ingen mikrofon");
    // Ingen enhed loeses ikke i privatlivsindstillingerne — en knap derhen
    // ville sende brugeren det forkerte sted hen.
    expect(failure.systemSettings).toBe(false);
  });

  it("naevner at en anden app kan holde mikrofonen ved NotReadableError", () => {
    const failure = describeMicrophoneFailure(domError("NotReadableError"));
    expect(failure.message).toContain("andet program");
    expect(failure.canRetry).toBe(true);
  });

  it("markerer OverconstrainedError som ikke-genproevbar", () => {
    const failure = describeMicrophoneFailure(
      domError("OverconstrainedError"),
    );
    expect(failure.canRetry).toBe(false);
  });

  it("gengiver ukendte fejl raat i stedet for at opfinde en aarsag", () => {
    const failure = describeMicrophoneFailure(new Error("noget helt tredje"));
    expect(failure.message).toContain("noget helt tredje");
    expect(failure.systemSettings).toBe(false);
  });

  it("haandterer vaerdier der slet ikke er fejl", () => {
    expect(describeMicrophoneFailure(null).message).toBeTruthy();
    expect(describeMicrophoneFailure(undefined).message).toBeTruthy();
    expect(describeMicrophoneFailure("bare en streng").message).toContain(
      "bare en streng",
    );
  });

  it("laekker ikke det engelske DOMException-navn ud i de oversatte beskeder", () => {
    for (const name of [
      "NotAllowedError",
      "NotFoundError",
      "NotReadableError",
      "OverconstrainedError",
      "SecurityError",
    ]) {
      expect(describeMicrophoneFailure(domError(name)).message).not.toContain(
        name,
      );
    }
  });
});

describe("MicrophoneError", () => {
  it("baerer den oversatte tekst men beholder originalen som cause", () => {
    const original = domError("NotAllowedError");
    const error = new MicrophoneError(
      describeMicrophoneFailure(original),
      original,
    );
    expect(error.message).toContain("Privatliv");
    expect(error.systemSettings).toBe(true);
    // Diagnostik maa ikke miste DOMException-navnet — kun det brugeren LAESER
    // er oversat.
    expect((error.cause as Error).name).toBe("NotAllowedError");
  });
});
