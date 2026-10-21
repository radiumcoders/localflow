import levelsJson from "./data/levels.json";
import timingsJson from "./data/timings.json";
import { FPS } from "./theme";

type Clip = { duration: number; timed: { text: string; start: number }[] };
const TIMINGS = timingsJson as unknown as Record<string, Clip>;
const LEVELS = levelsJson as Record<string, number[]>;

export const sec = (s: number) => Math.round(s * FPS);
/** A clip's length in frames. */
export const clipFrames = (id: string) => Math.ceil(TIMINGS[id].duration * FPS);
/** The clip's words with the frame each starts at. */
export const words = (id: string) => TIMINGS[id].timed.map((w) => ({ text: w.text, at: w.start * FPS }));
/** The microphone's last `n` levels, `frame` frames into the clip (0 outside it). */
export const levelsAt = (id: string, frame: number, n = 22) => {
  const l = LEVELS[id] ?? [];
  return Array.from({ length: n }, (_, i) => l[Math.floor(frame) - n + 1 + i] ?? 0);
};
