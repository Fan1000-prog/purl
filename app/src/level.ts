// Mic level from the recorder's "audio-level" events (raw RMS, ~0.005
// whisper-quiet to ~0.3 loud) as the 0..1 the pill and the orb expect.
// Both components smooth it themselves.

export function micLevel(rms: number): number {
  return Math.min(1, Math.max(0, rms * 6));
}
