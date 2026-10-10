export interface HistoryEntry {
  ts: number;
  raw: string;
  cleaned: string;
  used_llm: boolean;
  record_ms: number;
  transcribe_ms: number;
  cleanup_ms: number;
  words: number;
}

export interface Stats {
  total_words: number;
  total_record_ms: number;
  total_sessions: number;
  llm_cleanups: number;
  per_day: Record<string, number>;
}

export interface Snippet {
  trigger: string;
  expansion: string;
}

export interface LearnedRule {
  from: string;
  to: string;
  before: string;
  after: string;
  created: number;
  hits: number;
  last_hit: number;
  /** Times this exact correction has been observed. */
  seen: number;
  /** False while a risky rule waits for its second sighting. */
  active: boolean;
}

export type CleanupMode = "off" | "fast" | "smart";
export type Trust = "granted" | "denied" | "not_required";
