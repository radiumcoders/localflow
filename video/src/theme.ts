import { loadFont as loadInter } from "@remotion/google-fonts/Inter";
import { loadFont as loadMono } from "@remotion/google-fonts/JetBrainsMono";

export const SANS = loadInter("normal", { weights: ["400", "500", "600", "700"], subsets: ["latin"] }).fontFamily;
export const MONO = loadMono("normal", { weights: ["400", "500"], subsets: ["latin"] }).fontFamily;

export const FPS = 30;

export const C = {
  paper: "#F5F4F0",
  card: "#FFFFFF",
  line: "#E4E1DA",
  ink: "#16151A",
  muted: "#6E6B73",
  faint: "#A8A4AE",
  // The app icon's violet (crates/xtask/src/icon.rs).
  violet: "#5B4CE6",
  violetDeep: "#281A78",
  violetBar: "#4A3AC8",
  violetSoft: "#ECEAFE",
  filler: "#FFE0E3",
  fillerInk: "#C2364A",
  ok: "#22A35A",
};
