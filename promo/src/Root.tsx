import { Composition } from "remotion";
import { Intro } from "./Intro";
import manifest from "./manifest.generated.json";

export const RemotionRoot: React.FC = () => (
  <Composition
    id="ProductIntro"
    component={Intro}
    durationInFrames={manifest.total}
    fps={manifest.fps}
    width={manifest.width}
    height={manifest.height}
  />
);
