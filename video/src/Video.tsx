import React from "react";
import { AbsoluteFill, Easing, interpolate, Sequence, staticFile, useCurrentFrame } from "remotion";
import { Audio } from "@remotion/media";
import { Dictation, dictationFrames, Messages, Notes, pasteFrame } from "./scenes/Dictation";
import { C, SANS } from "./theme";
import { clipFrames, sec } from "./timing";
import { Icon } from "./ui/parts";

// What the app made of the two clips (`localflow transcribe` with Claude Haiku 5.5),
// word for word.
const MESSAGE = "Can we move the design review to Friday at 3, and ask Priya to bring the pricing slides?";
const LIST_TITLE = "My tasks today:";
const LIST = ["Buy milk", "Call the bank", "Fix the login bug"];

const FADE = 12;

/** A scene that fades and drifts in, and fades out at its end. */
const Scene: React.FC<{ dur: number; children: React.ReactNode }> = ({ dur, children }) => {
  const f = useCurrentFrame();
  const o = interpolate(f, [0, FADE, dur - FADE, dur], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const y = interpolate(f, [0, FADE * 2], [24, 0], { extrapolateRight: "clamp", easing: Easing.out(Easing.cubic) });
  return <AbsoluteFill style={{ opacity: o, transform: `translateY(${y}px)` }}>{children}</AbsoluteFill>;
};

const Narration: React.FC<{ clip: string; at?: number }> = ({ clip, at = 0 }) => (
  <Sequence from={at} layout="none">
    <Audio src={staticFile(`audio/${clip}.wav`)} />
  </Sequence>
);

const Big: React.FC<{ children: React.ReactNode; style?: React.CSSProperties }> = ({ children, style }) => (
  <div style={{ fontFamily: SANS, fontSize: 92, fontWeight: 700, letterSpacing: -2.5, color: C.ink, ...style }}>
    {children}
  </div>
);

/** "You think faster than you type.", typed slower than it's said. */
const Hook: React.FC = () => {
  const f = useCurrentFrame();
  const line = "You think faster than you type.";
  const n = Math.floor(interpolate(f, [6, 70], [0, line.length], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }));
  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center" }}>
      <Narration clip="n_hook" at={4} />
      <Big>
        {line.slice(0, n)}
        <span style={{ color: C.violet, opacity: Math.floor(f / 12) % 2 ? 0.2 : 1 }}>|</span>
      </Big>
    </AbsoluteFill>
  );
};

const Intro: React.FC = () => {
  const f = useCurrentFrame();
  const first = interpolate(f, [sec(1.0), sec(1.25)], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const messy = interpolate(f, [sec(1.25), sec(1.6)], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.out(Easing.back(1.6)) });
  const meant = interpolate(f, [sec(2.6), sec(2.95)], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center" }}>
      <Narration clip="n_intro" />
      <Big style={{ position: "absolute", opacity: first }}>So talk instead.</Big>
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 40, opacity: messy }}>
        <div style={{ transform: `scale(${0.85 + 0.15 * messy})` }}>
          <Icon size={132} />
        </div>
        <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 6 }}>
          <Big>Say it messy.</Big>
          <Big style={{ color: C.violet, opacity: meant, transform: `translateY(${(1 - meant) * 16}px)` }}>Get what you meant.</Big>
        </div>
      </div>
    </AbsoluteFill>
  );
};

const ListTitle: React.FC = () => (
  <AbsoluteFill style={{ alignItems: "center", justifyContent: "center" }}>
    <Narration clip="n_list" at={4} />
    <Big>
      Ramble a list. <span style={{ color: C.violet }}>Get a list.</span>
    </Big>
  </AbsoluteFill>
);

const Outro: React.FC = () => {
  const f = useCurrentFrame();
  const sub = interpolate(f, [sec(0.2), sec(0.6)], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  // With "Coming soon" in the narration (it starts 6 frames in).
  const soon = interpolate(f, [6 + sec(4.4), 6 + sec(4.8)], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.out(Easing.back(1.8)),
  });
  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", gap: 36 }}>
      <Narration clip="n_outro" at={6} />
      <Icon size={132} />
      <Big style={{ fontSize: 80, letterSpacing: -2 }}>Talk, and get clean text where you type.</Big>
      <div style={{ fontFamily: SANS, fontSize: 28, color: C.muted, opacity: sub }}>
        Private by default · macOS · Windows · Linux
      </div>
      <div
        style={{
          marginTop: 8,
          padding: "14px 30px",
          borderRadius: 999,
          background: C.violet,
          color: "#fff",
          fontFamily: SANS,
          fontSize: 32,
          fontWeight: 600,
          letterSpacing: 0.5,
          opacity: soon,
          transform: `scale(${0.8 + 0.2 * soon})`,
        }}
      >
        Coming soon
      </div>
    </AbsoluteFill>
  );
};

const MessageDemo: React.FC = () => (
  <Dictation
    clip="d_msg"
    title="Messages"
    after="n_after"
    marks={{ 0: "filler", 1: "filler", 6: "filler", 10: "change", 11: "change", 12: "change", 13: "change", 14: "change", 19: "filler" }}
    app={(pasted) => <Messages pasted={pasted} text={MESSAGE} paste={pasteFrame("d_msg")} />}
  />
);

const ListDemo: React.FC = () => (
  <Dictation
    clip="d_list"
    title="Notes"
    marks={{ 3: "filler", 4: "list", 7: "list", 11: "filler", 12: "list" }}
    app={(pasted) => <Notes pasted={pasted} title={LIST_TITLE} items={LIST} paste={pasteFrame("d_list")} />}
  />
);

export const SCENES: { key: string; dur: number; C: React.FC }[] = [
  { key: "hook", dur: sec(2.9), C: Hook },
  { key: "intro", dur: clipFrames("n_intro") + sec(0.8), C: Intro },
  { key: "message", dur: dictationFrames("d_msg", "n_after"), C: MessageDemo },
  { key: "listTitle", dur: clipFrames("n_list") + sec(0.2), C: ListTitle },
  { key: "list", dur: dictationFrames("d_list"), C: ListDemo },
  { key: "outro", dur: clipFrames("n_outro") + sec(1.6), C: Outro },
];

export const TOTAL = SCENES.reduce((t, s) => t + s.dur, 0);

export const Video: React.FC = () => {
  let at = 0;
  return (
    <AbsoluteFill style={{ background: C.paper }}>
      {SCENES.map(({ key, dur, C: S }) => {
        const from = at;
        at += dur;
        return (
          <Sequence key={key} from={from} durationInFrames={dur}>
            <Scene dur={dur}>
              <S />
            </Scene>
          </Sequence>
        );
      })}
    </AbsoluteFill>
  );
};
