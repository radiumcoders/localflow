import React from "react";
import { Composition } from "remotion";
import { FPS } from "./theme";
import { TOTAL, Video } from "./Video";
import { VIZ_TOTAL, VisualizeVideo } from "./Visualize";

export const RemotionRoot: React.FC = () => (
  <>
    <Composition id="Dictation" component={Video} durationInFrames={TOTAL} fps={FPS} width={1920} height={1080} />
    <Composition id="Visualize" component={VisualizeVideo} durationInFrames={VIZ_TOTAL} fps={FPS} width={1920} height={1080} />
  </>
);
