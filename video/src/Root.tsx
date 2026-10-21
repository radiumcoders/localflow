import React from "react";
import { Composition } from "remotion";
import { FPS } from "./theme";
import { TOTAL, Video } from "./Video";

export const RemotionRoot: React.FC = () => (
  <Composition id="Dictation" component={Video} durationInFrames={TOTAL} fps={FPS} width={1920} height={1080} />
);
