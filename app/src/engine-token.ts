/** The dropdown's option values are `engine:model` tokens. This is the only
 *  parser for them: the token never crosses the IPC boundary — `save_settings`
 *  takes `ai_engine` plus per-engine model fields, and the key commands take a
 *  bare engine name — so it exists purely to turn a selected option back into
 *  those structured fields. Keep the fail-safe direction: an unknown or missing
 *  prefix resolves to `local`, never to a cloud engine. */

export type Engine = "local" | "claude" | "gemini";

export interface EngineModel {
  engine: Engine;
  model: string;
}

const ENGINES: Engine[] = ["local", "claude", "gemini"];

/** Splits on the FIRST colon only — local model ids contain one
 *  ("qwen2.5:3b"). An unknown or missing prefix resolves to local, matching
 *  the Rust side's fail-safe onto the private path. */
export function splitToken(token: string): EngineModel {
  const at = token.indexOf(":");
  if (at > 0) {
    const head = token.slice(0, at);
    if ((ENGINES as string[]).includes(head)) {
      return { engine: head as Engine, model: token.slice(at + 1) };
    }
  }
  return { engine: "local", model: token };
}

export function makeToken(engine: Engine, model: string): string {
  return `${engine}:${model}`;
}

export function providerLabel(engine: Engine): string {
  return engine === "claude" ? "Claude" : engine === "gemini" ? "Gemini" : "Local";
}
