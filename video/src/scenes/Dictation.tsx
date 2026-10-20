import React from "react";
import { AbsoluteFill, Easing, interpolate, Sequence, staticFile, useCurrentFrame } from "remotion";
import { Audio } from "@remotion/media";
import { C, SANS } from "../theme";
import { clipFrames, levelsAt, words } from "../timing";
import { Pill } from "../ui/Pill";
import { AppWindow, Caption, Keys, type Mark, SaidCard } from "../ui/parts";

/** Before the key goes down, and how long each step after it shows (frames). */
export const LEAD = 24;
const TRANSCRIBING = 18;
const CLEANING = 36;
/** How long the result shows: through a line said over it (`after`), or a short beat. */
const holdFor = (after?: string) => (after ? 12 + clipFrames(after) + 18 : 70);

export const dictationFrames = (clip: string, after?: string) =>
  LEAD + clipFrames(clip) + TRANSCRIBING + CLEANING + holdFor(after);

/** One dictation into an app: hold the hotkey and talk, the pill listens, then
 *  "Transcribing…", "Cleaning up…", and the cleaned text lands in the app. */
export const Dictation: React.FC<{
  clip: string;
  marks: Record<number, Mark>;
  title: string;
  app: (pasted: number) => React.ReactNode;
  /** A line said over the result. */
  after?: string;
}> = ({ clip, marks, title, app, after }) => {
  const frame = useCurrentFrame();
  const talk = clipFrames(clip);
  const up = LEAD + talk;
  const cleaning = up + TRANSCRIBING;
  const paste = cleaning + CLEANING;
  const said = words(clip).map((w, i) => ({ ...w, at: w.at + LEAD, mark: marks[i] }));
  const strike = interpolate(frame, [cleaning, cleaning + 20], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const pasted = interpolate(frame, [paste, paste + 8], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const pillIn = interpolate(frame, [LEAD, LEAD + 6, paste - 4, paste], [0, 1, 1, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.out(Easing.quad),
  });
  const state =
    frame < up
      ? { kind: "rec" as const, levels: levelsAt(clip, frame - LEAD), seconds: Math.max(0, frame - LEAD) / 30 }
      : { kind: "work" as const, text: frame < cleaning ? "Transcribing" : "Cleaning up" };
  return (
    <AbsoluteFill style={{ fontFamily: SANS }}>
      <Sequence from={LEAD} layout="none">
        <Audio src={staticFile(`audio/${clip}.wav`)} />
      </Sequence>
      <Sequence from={paste} layout="none">
        <Audio src={staticFile("audio/paste.wav")} />
      </Sequence>
      {after && (
        <Sequence from={paste + 12} layout="none">
          <Audio src={staticFile(`audio/${after}.wav`)} />
        </Sequence>
      )}
      <div style={{ position: "absolute", left: 110, top: 150 }}>
        <SaidCard words={said} frame={frame} strike={strike} width={700} />
      </div>
      <div style={{ position: "absolute", left: 870, top: 150 }}>
        <AppWindow title={title} width={940} height={640}>
          {app(pasted)}
        </AppWindow>
      </div>
      <div style={{ position: "absolute", left: 110, top: 60, display: "flex", alignItems: "center", gap: 22 }}>
        <Keys keys={["Ctrl", "Shift", "Space"]} down={frame >= LEAD && frame < up} />
        <Caption style={{ opacity: frame >= LEAD && frame < up ? 1 : 0.5 }}>
          {frame < up ? "HOLD AND TALK" : "LET GO"}
        </Caption>
      </div>
      <div
        style={{
          position: "absolute",
          bottom: 56,
          width: "100%",
          display: "flex",
          justifyContent: "center",
          opacity: pillIn,
          transform: `translateY(${(1 - pillIn) * 20}px)`,
        }}
      >
        <Pill state={state} tick={frame} scale={1.9} />
      </div>
    </AbsoluteFill>
  );
};

/** How freshly pasted text looks: a violet wash that fades. */
export const pastedStyle = (pasted: number, frame: number, paste: number): React.CSSProperties => ({
  opacity: pasted,
  background: `rgba(91,76,230,${0.14 * interpolate(frame - paste, [0, 40], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })})`,
  borderRadius: 8,
});

export const pasteFrame = (clip: string) => LEAD + clipFrames(clip) + TRANSCRIBING + CLEANING;

/** The chat app the first dictation goes to. */
export const Messages: React.FC<{ pasted: number; text: string; paste: number }> = ({ pasted, text, paste }) => {
  const frame = useCurrentFrame();
  return (
    <AbsoluteFill style={{ display: "flex", flexDirection: "column" }}>
      <div style={{ flex: 1, padding: "32px 36px", display: "flex", flexDirection: "column", gap: 14 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 14, marginBottom: 10 }}>
          <div
            style={{
              width: 46,
              height: 46,
              borderRadius: 99,
              background: "#F2C6A0",
              color: "#7A4317",
              fontWeight: 700,
              fontSize: 20,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            P
          </div>
          <div style={{ fontSize: 22, fontWeight: 600, color: C.ink }}>Priya</div>
        </div>
        <div
          style={{
            alignSelf: "flex-start",
            maxWidth: 560,
            background: "#F1F0EC",
            borderRadius: "20px 20px 20px 6px",
            padding: "16px 20px",
            fontSize: 24,
            color: C.ink,
          }}
        >
          Are we still on for the design review on Thursday?
        </div>
      </div>
      <div style={{ padding: 24, borderTop: `1px solid ${C.line}` }}>
        <div
          style={{
            minHeight: 64,
            border: `1.5px solid ${pasted > 0 ? C.violet : C.line}`,
            borderRadius: 16,
            padding: "16px 20px",
            fontSize: 25,
            lineHeight: 1.4,
            color: C.ink,
          }}
        >
          {pasted > 0 ? (
            <span style={pastedStyle(pasted, frame, paste)}>{text}</span>
          ) : (
            <span style={{ color: C.faint }}>
              <span style={{ opacity: Math.floor(frame / 15) % 2 ? 0 : 1, color: C.ink }}>|</span> Message Priya
            </span>
          )}
        </div>
      </div>
    </AbsoluteFill>
  );
};

/** The notes app the list goes to. */
export const Notes: React.FC<{ pasted: number; title: string; items: string[]; paste: number }> = ({
  pasted,
  title,
  items,
  paste,
}) => {
  const frame = useCurrentFrame();
  return (
    <AbsoluteFill style={{ padding: "40px 52px", color: C.ink }}>
      <div style={{ fontSize: 40, fontWeight: 700, marginBottom: 24 }}>Monday</div>
      {pasted > 0 ? (
        <div style={{ ...pastedStyle(pasted, frame, paste), fontSize: 30, lineHeight: 1.65, padding: "4px 8px" }}>
          <div>{title}</div>
          {items.map((it, i) => (
            <div key={it}>
              {i + 1}. {it}
            </div>
          ))}
        </div>
      ) : (
        <div style={{ fontSize: 30, color: C.ink, opacity: Math.floor(frame / 15) % 2 ? 0 : 1 }}>|</div>
      )}
    </AbsoluteFill>
  );
};
