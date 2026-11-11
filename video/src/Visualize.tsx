import React from "react";
import { AbsoluteFill, Easing, Img, interpolate, Sequence, staticFile, useCurrentFrame } from "remotion";
import { Audio } from "@remotion/media";
import { Dictation, dictationFrames, pasteFrame, pastedStyle } from "./scenes/Dictation";
import { C, MONO, SANS } from "./theme";
import { clipFrames, sec } from "./timing";
import { AppWindow, Icon } from "./ui/parts";

// What localflow made of the voiced clip: `localflow transcribe` heard it word for word,
// and `localflow visualize` (Claude Haiku 5.5) drew public/viz/deploy.png from it; this is
// its Mermaid (public/viz/deploy.mmd).
const MERMAID = `flowchart TD
    N1(["Open a pull request"])
    N2["CI runs the tests"]
    N3{"Tests pass?"}
    N4["Someone reviews it"]
    N5(["Merge"])
    N6["Fix and push again"]
    N1 --> N2
    N2 --> N3
    N3 -->|Yes| N4
    N3 -->|No| N6
    N6 -->|Push again| N2
    N4 --> N5`;

const FADE = 12;

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

const pop = (f: number, at: number) =>
  interpolate(f, [at, at + 10], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.out(Easing.back(1.6)) });

/** "Say it. See it.": a sentence, then the shape of it. */
const Hook: React.FC = () => {
  const f = useCurrentFrame();
  const see = pop(f, 4 + sec(1.23));
  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center" }}>
      <Narration clip="n_viz_hook" at={4} />
      <Big>
        Say it.{" "}
        <span style={{ color: C.violet, opacity: see, display: "inline-block", transform: `scale(${0.85 + 0.15 * see})` }}>
          See it.
        </span>
      </Big>
    </AbsoluteFill>
  );
};

/** The team chat the diagram is pasted into, as a picture. */
const Chat: React.FC<{ pasted: number; paste: number }> = ({ pasted, paste }) => {
  const frame = useCurrentFrame();
  return (
    <AbsoluteFill style={{ display: "flex", flexDirection: "column" }}>
      <div style={{ padding: "26px 34px 0", display: "flex", gap: 14 }}>
        <div
          style={{
            width: 46,
            height: 46,
            borderRadius: 12,
            background: "#BFE3D0",
            color: "#1F6B45",
            fontWeight: 700,
            fontSize: 20,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            flexShrink: 0,
          }}
        >
          S
        </div>
        <div>
          <div style={{ fontSize: 20, fontWeight: 700, color: C.ink }}>
            Sam <span style={{ fontWeight: 400, fontSize: 16, color: C.faint }}>10:42</span>
          </div>
          <div style={{ fontSize: 23, color: C.ink, marginTop: 4 }}>New here: how does a change get to main?</div>
        </div>
      </div>
      <div style={{ flex: 1 }} />
      <div style={{ padding: 22, borderTop: `1px solid ${C.line}` }}>
        <div
          style={{
            border: `1.5px solid ${pasted > 0 ? C.violet : C.line}`,
            borderRadius: 16,
            padding: 16,
            fontSize: 23,
            color: C.faint,
          }}
        >
          {pasted > 0 ? (
            <div style={{ ...pastedStyle(pasted, frame, paste), padding: 8, display: "inline-block" }}>
              <Img
                src={staticFile("viz/deploy.png")}
                style={{
                  height: 372,
                  borderRadius: 10,
                  border: `1px solid ${C.line}`,
                  transform: `scale(${0.94 + 0.06 * pasted})`,
                  transformOrigin: "left bottom",
                }}
              />
              <div style={{ fontSize: 15, color: C.muted, marginTop: 6, fontFamily: MONO }}>flowchart.png</div>
            </div>
          ) : (
            <span>
              <span style={{ opacity: Math.floor(frame / 15) % 2 ? 0 : 1, color: C.ink }}>|</span> Message #eng
            </span>
          )}
        </div>
      </div>
    </AbsoluteFill>
  );
};

const ChatDemo: React.FC = () => (
  <Dictation
    clip="d_viz"
    title="#eng"
    working="Drawing a flowchart"
    marks={{ 0: "list" }}
    app={(pasted) => <Chat pasted={pasted} paste={pasteFrame("d_viz")} />}
  />
);

/** The same diagram dictated into an editor: its Mermaid, in a fenced block. */
const Editor: React.FC = () => {
  const f = useCurrentFrame();
  const paste = sec(0.7);
  const pasted = interpolate(f, [paste, paste + 8], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const lines = ["```mermaid", ...MERMAID.split("\n"), "```"];
  const kw = (l: string) => /^(flowchart|```)/.test(l.trim());
  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center" }}>
      <Narration clip="n_viz_editor" at={4} />
      <Sequence from={paste} layout="none">
        <Audio src={staticFile("audio/paste.wav")} />
      </Sequence>
      <AppWindow title="docs/deploy.md" width={1240} height={760}>
        <div style={{ position: "absolute", inset: 0, background: "#1C1B22", padding: "28px 40px", fontFamily: MONO, fontSize: 22, lineHeight: 1.5, fontVariantLigatures: "none" }}>
          <div style={{ color: "#8B8A96" }}># Deploy</div>
          <div style={{ height: 14 }} />
          {pasted > 0 ? (
            <div style={{ ...pastedStyle(pasted, f, paste), padding: "4px 10px" }}>
              {lines.map((l, i) => (
                <div key={i} style={{ whiteSpace: "pre", color: kw(l) ? "#B4A9FF" : "#E8E6EE" }}>
                  {l.replace(/^ {4}/, "  ")}
                </div>
              ))}
            </div>
          ) : (
            <div style={{ color: "#E8E6EE", opacity: Math.floor(f / 15) % 2 ? 0 : 1 }}>|</div>
          )}
        </div>
      </AppWindow>
    </AbsoluteFill>
  );
};

const Outro: React.FC = () => {
  const f = useCurrentFrame();
  const beta = pop(f, 6 + sec(2.23));
  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", gap: 36 }}>
      <Narration clip="n_viz_outro" at={6} />
      <Icon size={132} />
      <Big style={{ fontSize: 84, letterSpacing: -2 }}>Talk, and get the diagram.</Big>
      <div
        style={{
          padding: "14px 30px",
          borderRadius: 999,
          background: C.violet,
          color: "#fff",
          fontFamily: SANS,
          fontSize: 32,
          fontWeight: 600,
          letterSpacing: 0.5,
          opacity: beta,
          transform: `scale(${0.8 + 0.2 * beta})`,
        }}
      >
        Now in beta
      </div>
    </AbsoluteFill>
  );
};

export const VIZ_SCENES: { key: string; dur: number; C: React.FC }[] = [
  { key: "hook", dur: clipFrames("n_viz_hook") + sec(0.7), C: Hook },
  { key: "chat", dur: dictationFrames("d_viz"), C: ChatDemo },
  { key: "editor", dur: clipFrames("n_viz_editor") + sec(1.4), C: Editor },
  { key: "outro", dur: clipFrames("n_viz_outro") + sec(1.6), C: Outro },
];

export const VIZ_TOTAL = VIZ_SCENES.reduce((t, s) => t + s.dur, 0);

export const VisualizeVideo: React.FC = () => {
  let at = 0;
  return (
    <AbsoluteFill style={{ background: C.paper }}>
      {VIZ_SCENES.map(({ key, dur, C: S }) => {
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
