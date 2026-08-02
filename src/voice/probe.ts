import sttProbeUrl from "../assets/stt-probe-da.pcm?url";
import type { VoiceRoutes } from "../types";
import { createOpenAiSttClient, STT_DOMAIN_PROMPT } from "./stt";

export async function probeSttRoute(routes: VoiceRoutes): Promise<string> {
  const route = routes.stt;
  // Ruten er altid realtime siden OpenRouter-STT blev slettet (2026-07-29).
  // Proben bygger stadig klienten ud fra RUTEN og ikke ud fra konstanter, saa
  // en fremtidig udbyder arver testknappen uden at nogen skal huske det her.
  const client = createOpenAiSttClient({
    model: route.model,
    endpoint: route.endpoint,
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
