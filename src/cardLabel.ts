/**
 * Kortets visningsnavn — det eneste sted reglen "brugeren læser dansk" bor.
 *
 * Wire-identifieren er og bliver `card-N` (registry.rs:293). Den er den stabile
 * nøgle for spawn_card/write_pty/kill_card/get_card_state, den står i MCP-fladen
 * agenterne ser (`card_pair` → "card-3") og i x-talminal-session-headeren. Kun
 * det brugeren LÆSER oversættes — omdøbes identifieren, brydes hele den flade.
 *
 * Toml-seedede kort beholder deres eget navn (registry.rs:24) og har derfor
 * ikke card-N-formen. De vises uændret: at gætte et nummer for dem ville være
 * en påstand vi ikke har dækning for.
 */
export function cardLabel(name: string): string {
  const match = /^card-(\d+)$/.exec(name);
  return match === null ? name : `Kort ${match[1]}`;
}

export default cardLabel;
