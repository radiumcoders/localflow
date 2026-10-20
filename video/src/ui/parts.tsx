import React from "react";
import { C, MONO, SANS } from "../theme";

/** The app icon (crates/xtask/src/icon.rs): a violet square with the pill's level bars. */
export const Icon: React.FC<{ size: number }> = ({ size }) => (
  <svg width={size} height={size} viewBox="0 0 1024 1024">
    <defs>
      <linearGradient id="lf-bg" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stopColor="rgb(91,76,230)" />
        <stop offset="1" stopColor="rgb(40,26,120)" />
      </linearGradient>
    </defs>
    <rect x={40} y={40} width={944} height={944} rx={220} fill="url(#lf-bg)" />
    <rect x={170} y={362} width={684} height={300} rx={150} fill="#fff" />
    {[70, 150, 220, 150, 70].map((h, i) => (
      <rect key={i} x={512 + (i - 2) * 92 - 24} y={512 - h / 2} width={48} height={h} rx={24} fill="rgb(74,58,200)" />
    ))}
  </svg>
);

/** A plain app window: title bar with three dots, then the body. */
export const AppWindow: React.FC<{
  title: string;
  width: number;
  height: number;
  children: React.ReactNode;
  style?: React.CSSProperties;
}> = ({ title, width, height, children, style }) => (
  <div
    style={{
      width,
      height,
      background: C.card,
      borderRadius: 18,
      border: `1px solid ${C.line}`,
      boxShadow: "0 40px 80px -40px rgba(30,20,80,0.35), 0 2px 6px rgba(0,0,0,0.04)",
      overflow: "hidden",
      display: "flex",
      flexDirection: "column",
      fontFamily: SANS,
      ...style,
    }}
  >
    <div
      style={{
        height: 52,
        display: "flex",
        alignItems: "center",
        padding: "0 20px",
        gap: 9,
        borderBottom: `1px solid ${C.line}`,
        background: "#FAFAF8",
        flexShrink: 0,
      }}
    >
      {["#FF5F57", "#FEBC2E", "#28C840"].map((c) => (
        <div key={c} style={{ width: 13, height: 13, borderRadius: 99, background: c }} />
      ))}
      <div style={{ flex: 1, textAlign: "center", fontSize: 17, fontWeight: 600, color: C.muted, marginRight: 60 }}>
        {title}
      </div>
    </div>
    <div style={{ flex: 1, position: "relative" }}>{children}</div>
  </div>
);

/** Keys held down: each lifts when `down` is false. */
export const Keys: React.FC<{ keys: string[]; down: boolean; style?: React.CSSProperties }> = ({ keys, down, style }) => (
  <div style={{ display: "flex", gap: 10, alignItems: "center", fontFamily: SANS, ...style }}>
    {keys.map((k, i) => (
      <React.Fragment key={k}>
        {i > 0 && <span style={{ color: C.faint, fontSize: 22, fontWeight: 600 }}>+</span>}
        <div
          style={{
            minWidth: 64,
            padding: "12px 18px",
            borderRadius: 12,
            background: down ? C.violetSoft : C.card,
            border: `1.5px solid ${down ? C.violet : C.line}`,
            boxShadow: down ? `0 1px 0 ${C.violet}` : `0 4px 0 ${C.line}`,
            transform: `translateY(${down ? 3 : 0}px)`,
            fontSize: 22,
            fontWeight: 600,
            color: down ? C.violet : C.ink,
            textAlign: "center",
          }}
        >
          {k}
        </div>
      </React.Fragment>
    ))}
  </div>
);

export type Mark = "filler" | "change" | "list" | undefined;

/** What you said, word by word as you say it; marked words get struck through once
 *  `strike` (0..1) runs, the way the cleanup drops them. */
export const SaidCard: React.FC<{
  words: { text: string; at: number; mark?: Mark }[];
  frame: number;
  strike: number;
  width: number;
}> = ({ words, frame, strike, width }) => (
  <div
    style={{
      width,
      padding: "34px 38px",
      borderRadius: 20,
      background: C.card,
      border: `1px solid ${C.line}`,
      fontFamily: SANS,
      boxShadow: "0 30px 60px -40px rgba(30,20,80,0.3)",
    }}
  >
    <div style={{ fontSize: 16, fontWeight: 600, letterSpacing: 1.5, color: C.faint, marginBottom: 18 }}>YOU SAY</div>
    <div style={{ fontSize: 34, lineHeight: 1.45, color: C.ink, fontWeight: 500 }}>
      {words.map((w, i) => {
        const shown = frame >= w.at;
        const color = w.mark === "list" ? C.violet : C.fillerInk;
        const bg = w.mark === "list" ? C.violetSoft : C.filler;
        return (
          <span
            key={i}
            style={{
              opacity: shown ? 1 : 0.12,
              color: w.mark && shown ? color : undefined,
              background: w.mark && shown ? bg : undefined,
              borderRadius: 6,
              padding: w.mark ? "0 4px" : undefined,
              textDecoration: w.mark && w.mark !== "list" && strike > 0 ? "line-through" : undefined,
              textDecorationThickness: 3,
              textDecorationColor: `rgba(194,54,74,${strike})`,
            }}
          >
            {w.text}{" "}
          </span>
        );
      })}
    </div>
  </div>
);

export const Caption: React.FC<{ children: React.ReactNode; style?: React.CSSProperties }> = ({ children, style }) => (
  <div style={{ fontFamily: SANS, fontSize: 18, fontWeight: 600, letterSpacing: 1.5, color: C.faint, ...style }}>
    {children}
  </div>
);

export const Mono: React.FC<{ children: React.ReactNode; style?: React.CSSProperties }> = ({ children, style }) => (
  <span style={{ fontFamily: MONO, ...style }}>{children}</span>
);
