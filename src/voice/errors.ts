/**
 * Fejl-normalisering for stemme-laget.
 *
 * `catch (e)` giver `unknown`, men alle stemme-modulernes fejlveje vil have en
 * `Error` at raekke videre. Konverteringen stod byte-identisk i `pipeline.ts`,
 * `ptt.ts` og `realtime.ts`; her er den ét sted.
 */
export function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
