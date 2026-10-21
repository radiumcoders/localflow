import React from "react";
import { SANS } from "../theme";

/** localflow's pill (crates/app/src/ui/overlay.rs), drawn the same way: a red dot that
 *  pulses, 22 level bars and the time while you talk; "Transcribing…" and "Cleaning up…"
 *  after. `scale` enlarges it for the video. */
export type PillState = { kind: "rec"; levels: number[]; seconds: number } | { kind: "work"; text: string };

const BARS = 22;
const H = 40;

export const Pill: React.FC<{ state: PillState; tick: number; scale?: number; style?: React.CSSProperties }> = ({
  state,
  tick,
  scale = 1,
  style,
}) => {
  let content: React.ReactNode;
  if (state.kind === "rec") {
    const levels = Array.from({ length: BARS }, (_, i) => state.levels[state.levels.length - BARS + i] ?? 0);
    const speaking = levels.slice(-4).some((l) => l > 0.35);
    const dot = speaking || Math.floor(tick / 8) % 2 === 1 ? 11 : 8;
    const s = Math.floor(state.seconds);
    content = (
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <div style={{ width: 12, height: 12, display: "flex", alignItems: "center", justifyContent: "center" }}>
          <div style={{ width: dot, height: dot, borderRadius: 99, background: "#ef4444" }} />
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 3, height: H }}>
          {levels.map((l, i) => (
            <div
              key={i}
              style={{ width: 3, height: 2 + l * (H - 16), borderRadius: 1.5, background: l > 0.2 ? "#f4f4f5" : "#8b8b94" }}
            />
          ))}
        </div>
        <div style={{ width: 30, textAlign: "right", color: "#8b8b94", fontFeatureSettings: "'tnum'" }}>
          {Math.floor(s / 60)}:{String(s % 60).padStart(2, "0")}
        </div>
      </div>
    );
  } else {
    const dots = ".".repeat(1 + (Math.floor(tick / 6) % 3));
    content = (
      <div style={{ display: "flex", alignItems: "center", color: "#f4f4f5" }}>
        {state.text}
        <span style={{ width: 14 }}>{dots}</span>
      </div>
    );
  }
  return (
    <div
      style={{
        height: H,
        padding: "0 16px",
        display: "inline-flex",
        alignItems: "center",
        borderRadius: 999,
        background: "#18181b",
        border: "1px solid #2e2e33",
        fontFamily: SANS,
        fontSize: 13,
        whiteSpace: "nowrap",
        boxShadow: "0 12px 32px -12px rgba(0,0,0,0.45)",
        zoom: scale,
        ...style,
      }}
    >
      {content}
    </div>
  );
};
