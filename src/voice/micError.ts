/**
 * Oversætter `getUserMedia`-fejl til noget brugeren kan handle på.
 *
 * HUD'en viser fejlens `message` ordret (App.tsx' `onError`), så teksten HER er
 * det eneste brugeren får at vide. Rå browserfejl som "NotAllowedError:
 * Permission denied" fortæller hverken hvad der gik galt eller hvad man gør —
 * og mikrofon-adgang kan blokeres tre helt forskellige steder, med hver sin vej
 * ud. Derfor navngives stedet, ikke symptomet.
 *
 * Bemærk at `NotAllowedError` dækker BÅDE en afvist WebView2-prompt og en
 * slukket Windows-privatlivsindstilling. Browseren skelner dem ikke, så teksten
 * kan det heller ikke — den peger på den af de to der oftest er årsagen for en
 * ny bruger, og nævner den anden.
 */
export type MicrophoneFailure = {
  /** Kort, handlingsanvisende besked — det HUD'en viser. */
  message: string;
  /** Hjælper recovery-fladen: giver et nyt forsøg overhovedet mening? */
  canRetry: boolean;
  /** Peger fejlen på Windows' egen privatlivsindstilling? */
  systemSettings: boolean;
};

/** DOMException-navnet, uden at antage at fejlen ER en DOMException. */
function errorName(error: unknown): string {
  if (typeof error === "object" && error !== null && "name" in error) {
    const name = (error as { name: unknown }).name;
    if (typeof name === "string") return name;
  }
  return "";
}

export function describeMicrophoneFailure(error: unknown): MicrophoneFailure {
  switch (errorName(error)) {
    case "NotAllowedError":
    case "PermissionDeniedError": // ældre navn, samme sag
      return {
        message:
          "Mikrofonen er blokeret. Åbn Windows-indstillinger → Privatliv og " +
          "sikkerhed → Mikrofon, og slå adgang til for skrivebordsapps. " +
          "Er den allerede slået til, så genstart Talminal.",
        canRetry: true,
        systemSettings: true,
      };
    case "NotFoundError":
    case "DevicesNotFoundError":
      return {
        message:
          "Ingen mikrofon fundet. Tilslut en, eller aktivér den i " +
          "Enhedshåndtering, og prøv igen.",
        canRetry: true,
        systemSettings: false,
      };
    case "NotReadableError":
    case "TrackStartError":
      return {
        message:
          "Mikrofonen kunne ikke åbnes — den er sandsynligvis optaget af et " +
          "andet program. Luk det, og prøv igen.",
        canRetry: true,
        systemSettings: false,
      };
    case "OverconstrainedError":
    case "ConstraintNotSatisfiedError":
      return {
        message:
          "Mikrofonen understøtter ikke det krævede lydformat. Vælg en anden " +
          "enhed som standard i Windows' lydindstillinger.",
        canRetry: false,
        systemSettings: true,
      };
    case "SecurityError":
      return {
        message:
          "Mikrofon-adgang blev afvist af sikkerhedsgrunde. Genstart " +
          "Talminal; sker det igen, er det en fejl i appen.",
        canRetry: true,
        systemSettings: false,
      };
    default: {
      // Ukendt fejl: gengiv den rå tekst frem for at opfinde en aarsag. En
      // forkert forklaring er vaerre end en uoversat.
      const raw =
        error instanceof Error ? error.message : String(error ?? "ukendt fejl");
      return {
        message: `Mikrofonen kunne ikke startes: ${raw}`,
        canRetry: true,
        systemSettings: false,
      };
    }
  }
}

/**
 * Fejl der bærer den oversatte tekst videre, men holder på originalen.
 * `cause` bevares, så konsol-diagnostik ikke mister DOMException-navnet —
 * kun det brugeren LÆSER er oversat.
 */
export class MicrophoneError extends Error {
  readonly canRetry: boolean;
  readonly systemSettings: boolean;

  constructor(failure: MicrophoneFailure, cause: unknown) {
    super(failure.message, { cause });
    this.name = "MicrophoneError";
    this.canRetry = failure.canRetry;
    this.systemSettings = failure.systemSettings;
  }
}
