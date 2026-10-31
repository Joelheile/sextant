import { TransitionSeries, linearTiming } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { useVideoConfig } from "remotion";
import { Ask } from "./scenes/Ask";
import { End } from "./scenes/End";
import { Skill } from "./scenes/Skill";
import { Week } from "./scenes/Week";

/** The README hero: where time goes, ask Claude, get a skill, then the pitch. */
export const Hero = () => {
  const { fps } = useVideoConfig();
  return (
    <TransitionSeries>
      <TransitionSeries.Sequence name="Week" durationInFrames={3.6 * fps}>
        <Week />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={fade()}
        timing={linearTiming({ durationInFrames: 10 })}
      />
      <TransitionSeries.Sequence name="Ask" durationInFrames={4.4 * fps}>
        <Ask />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={fade()}
        timing={linearTiming({ durationInFrames: 10 })}
      />
      <TransitionSeries.Sequence name="Skill" durationInFrames={3.6 * fps}>
        <Skill />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={fade()}
        timing={linearTiming({ durationInFrames: 10 })}
      />
      <TransitionSeries.Sequence name="End" durationInFrames={2.4 * fps}>
        <End />
      </TransitionSeries.Sequence>
    </TransitionSeries>
  );
};
