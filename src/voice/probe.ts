import sttProbeUrl from "../assets/stt-probe-da.pcm?url";
import type { VoiceRoutes } from "../types";
import { createOpenAiSttClient, STT_DOMAIN_PROMPT } from "./stt";

/**
 * `keywords` er et ANDET argument end ruten, fordi det er en brugerindstilling
 * og ikke en egenskab ved ruten. Proben SKAL have dem med: uden ville "Test
 * forbindelsen" svare groent paa en konfiguration brugeren aldrig taler paa,
 * og det er praecis den slags falske kvittering knappen findes for at undgaa.
 */
export async function probeSttRoute(
  routes: VoiceRoutes,
  keywords: readonly string[] = [],
): Promise<string> {
  const route = routes.stt;
  // Ruten er altid realtime siden OpenRouter-STT blev slettet (2026-07-29).
  // Proben bygger stadig klienten ud fra RUTEN og ikke ud fra konstanter, saa
  // en fremtidig udbyder arver testknappen uden at nogen skal huske det her.
  const client = createOpenAiSttClient({
    model: route.model,
    endpoint: route.endpoint,
    languageField: route.language_field,
    keywords: route.supports_keywords ? keywords : [],
    prompt: route.supports_domain_prompt ? STT_DOMAIN_PROMPT : null,
  });
  const pcm = await fetch(sttProbeUrl).then((response) => response.arrayBuffer());
  await client.start();
  try {
    client.pushAudio(pcm);
    return await client.stop();
  } catch (error) {
    client.abort();
    throw error;
  }
}
