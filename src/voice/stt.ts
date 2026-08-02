import { invoke } from "@tauri-apps/api/core";

export interface SttClient {
  start(): Promise<void>;
  pushAudio(chunk: ArrayBuffer): void;
  onPartial(cb: (text: string) => void): void;
  stop(): Promise<string>;
  abort(): void;
}

const FINAL_TIMEOUT_MS = 10_000;

export const STT_DOMAIN_PROMPT =
  "Dansk kommando til en canvas af nummererede kort: kort et, to, tre, fire, fem, seks, syv, otte, ni, ti. Typiske verber: luk, genstart, åbn, opret, send, sig til, bed. Ord som terminal, browser, projekt, canvas. Flere kommandoer kan kædes med og. Blandet dansk og engelsk kan forekomme.";

function bytesToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  const stride = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += stride) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + stride));
  }
  return btoa(binary);
}

function errorMessage(payload: unknown): string {
  if (typeof payload === "object" && payload !== null) {
    const error = (payload as { error?: unknown }).error;
    if (typeof error === "object" && error !== null) {
      const message = (error as { message?: unknown }).message;
      if (typeof message === "string") return message;
    }
  }
  return "OpenAI realtime transcription failed";
}

/** OpenAI GA realtime transcription over a browser-compatible WebSocket. */
export function createOpenAiSttClient(
  options: {
    model: string;
    endpoint: string;
    prompt?: string | null;
    mintSecret?: () => Promise<{ value: string; expires_at: number }>;
    createSocket?: (url: string, protocols: string[]) => WebSocket;
  },
): SttClient {
  const mintSecret =
    options.mintSecret ??
    (() =>
      invoke<{ value: string; expires_at: number }>(
        "mint_transcription_secret",
      ));
  const createSocket =
    options.createSocket ??
    ((url, protocols) => new WebSocket(url, protocols));
  let socket: WebSocket | null = null;
  let partial: (text: string) => void = () => undefined;
  let started = false;
  let starting = false;
  let terminalError: Error | null = null;
  let pendingFinal:
    | {
        socket: WebSocket;
        timeout: ReturnType<typeof setTimeout>;
        resolve: (text: string) => void;
        reject: (error: Error) => void;
      }
    | null = null;

  function closeSocket() {
    const current = socket;
    socket = null;
    started = false;
    if (current && current.readyState !== WebSocket.CLOSED) current.close();
  }

  function rejectFinal(ws: WebSocket, error: Error) {
    if (!pendingFinal || pendingFinal.socket !== ws) return;
    const final = pendingFinal;
    pendingFinal = null;
    clearTimeout(final.timeout);
    final.reject(error);
  }

  function resolveFinal(ws: WebSocket, transcript: string) {
    if (!pendingFinal || pendingFinal.socket !== ws) return;
    const final = pendingFinal;
    pendingFinal = null;
    clearTimeout(final.timeout);
    final.resolve(transcript);
  }

  function clearFinal(ws: WebSocket) {
    if (!pendingFinal || pendingFinal.socket !== ws) return;
    const final = pendingFinal;
    pendingFinal = null;
    clearTimeout(final.timeout);
  }

  return {
    async start() {
      if (socket !== null || started || starting) {
        throw new Error("STT session already started");
      }

      starting = true;
      try {
        const secret = await mintSecret();
        const apiKey = secret.value.trim();
        if (!apiKey) throw new Error("Missing STT API key");

        terminalError = null;
        const ws = createSocket(options.endpoint, [
          "realtime",
          `openai-insecure-api-key.${apiKey}`,
        ]);
        socket = ws;

        await new Promise<void>((resolve, reject) => {
          let opened = false;

          ws.onopen = () => {
            if (socket !== ws) return;
            opened = true;
            started = true;
            ws.send(
              JSON.stringify({
                type: "session.update",
                session: {
                  type: "transcription",
                  audio: {
                    input: {
                      format: { type: "audio/pcm", rate: 24_000 },
                      transcription: {
                        model: options.model,
                        language: "da",
                        ...(options.prompt === null
                          ? {}
                          : { prompt: options.prompt ?? STT_DOMAIN_PROMPT }),
                      },
                      turn_detection: null,
                    },
                  },
                },
              }),
            );
            resolve();
          };

          ws.onerror = () => {
            if (socket !== ws) return;
            const error = new Error("OpenAI realtime WebSocket error");
            terminalError = error;
            if (!opened) reject(error);
            rejectFinal(ws, error);
            closeSocket();
          };

          ws.onclose = () => {
            if (socket !== ws) return;
            socket = null;
            started = false;

            if (!opened) {
              const error =
                terminalError ??
                new Error("OpenAI realtime WebSocket closed before opening");
              terminalError = error;
              reject(error);
            } else if (!terminalError) {
              terminalError = new Error(
                "OpenAI realtime WebSocket closed before final transcript",
              );
            }

            rejectFinal(
              ws,
              terminalError ??
                new Error(
                  "OpenAI realtime WebSocket closed before final transcript",
                ),
            );
          };

          ws.onmessage = (event) => {
            if (socket !== ws) return;
            let message: {
              type?: string;
              delta?: string;
              transcript?: string;
              error?: unknown;
            };
            try {
              message = JSON.parse(String(event.data));
            } catch {
              return;
            }

            if (message.type === "error") {
              terminalError = new Error(errorMessage(message));
              rejectFinal(ws, terminalError);
              closeSocket();
              return;
            }

            if (
              message.type ===
                "conversation.item.input_audio_transcription.delta" &&
              typeof message.delta === "string"
            ) {
              partial(message.delta);
              return;
            }

            if (
              message.type ===
              "conversation.item.input_audio_transcription.completed"
            ) {
              resolveFinal(ws, message.transcript ?? "");
              closeSocket();
            }
          };
        });
      } finally {
        starting = false;
      }
    },
    abort() {
      const current = socket;
      if (!current) return;
      clearFinal(current);
      closeSocket();
    },

    pushAudio(chunk) {
      if (!socket || socket.readyState !== WebSocket.OPEN || !started) {
        throw new Error("STT session is not open");
      }
      socket.send(
        JSON.stringify({
          type: "input_audio_buffer.append",
          audio: bytesToBase64(chunk),
        }),
      );
    },

    onPartial(cb) {
      partial = cb;
    },

    async stop() {
      if (terminalError) {
        const error = terminalError;
        closeSocket();
        throw error;
      }
      if (!socket || !started) throw new Error("STT session is not open");
      if (pendingFinal) throw new Error("STT session is already finalizing");

      const ws = socket;
      const final = new Promise<string>((resolve, reject) => {
        const timeout = setTimeout(() => {
          if (!pendingFinal || pendingFinal.socket !== ws) return;
          const error = new Error(
            `Timed out waiting for final transcript after ${FINAL_TIMEOUT_MS} ms`,
          );
          terminalError = error;
          rejectFinal(ws, error);
          if (socket === ws) closeSocket();
        }, FINAL_TIMEOUT_MS);
        pendingFinal = { socket: ws, timeout, resolve, reject };
      });
      ws.send(JSON.stringify({ type: "input_audio_buffer.commit" }));

      try {
        return await final;
      } finally {
        clearFinal(ws);
        if (socket === ws) closeSocket();
      }
    },
  };
}
