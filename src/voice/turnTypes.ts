// Delte tur-typer. Boede oprindeligt i realtime.ts, men bruges af
// pipeline-vejen og HUD'en — og realtime-motoren forlader appen (spec
// 2026-07-28 §0), saa de skal have et hjem der ikke gaar med.
//
// Navnene beholder deres Realtime-praefiks med vilje: en omdoebning ville
// ripple gennem pipeline.ts, App.tsx, Hud.tsx og otte testfiler i samme
// diff som flytningen. Tag den separat, hvis den skal tages.

export type JsonObject = Record<string, unknown>;

export interface RealtimeToolCall {
  name: string;
  arguments: JsonObject;
  callId?: string;
}

export type RealtimeUiState =
  | "asleep"
  | "waking"
  | "awake"
  | "listening"
  | "processing"
  | "speaking"
  | "sleeping";

export interface RealtimeTurnCapture {
  ts: string;
  transcript: string;
  tool: { name: string; arguments: JsonObject };
  resolver: unknown;
  latency_ms: number;
  action_count: 0;
  commands?: Array<{ tool: { name: string; arguments: JsonObject }; resolver: unknown }>;
  command_count?: number;
}
