import { Composition } from "remotion";
import { Hero } from "./Hero";

export const RemotionRoot: React.FC = () => {
  return (
    <Composition
      id="Hero"
      component={Hero}
      durationInFrames={390}
      fps={30}
      width={1200}
      height={680}
    />
  );
};
