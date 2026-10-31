import { interpolate, useCurrentFrame } from "remotion";
import { Scene } from "../Scene";
import { clamp, color, easeOut } from "../theme";

const QUESTION = "What should I automate first?";
const ANSWER =
  "Start with invoices. You spend 1.5 h a week on them, the steps are the same every time, and every value is copied from the email.";

/** Shows `text` one character at a time from `start`, `perFrame` characters each frame. */
const typed = (text: string, frame: number, start: number, perFrame: number) =>
  text.slice(0, Math.max(0, Math.floor((frame - start) * perFrame)));

const Chat = () => {
  const frame = useCurrentFrame();
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 26,
        fontSize: 27,
        lineHeight: 1.45,
      }}
    >
      <div
        style={{
          alignSelf: "flex-end",
          minHeight: 39,
          padding: "14px 22px",
          borderRadius: 20,
          backgroundColor: color.subtle,
          fontWeight: 520,
          opacity: interpolate(frame, [8, 12], [0, 1], clamp),
        }}
      >
        {typed(QUESTION, frame, 10, 1.2)}
      </div>
      <div
        style={{
          alignSelf: "flex-start",
          display: "flex",
          alignItems: "center",
          gap: 10,
          fontSize: 20,
          fontWeight: 600,
          color: color.primary,
          padding: "8px 16px",
          borderRadius: 99,
          outline: `1.5px solid ${color.primary}`,
          opacity: interpolate(frame, [42, 50], [0, 1], clamp),
          translate: interpolate(frame, [42, 54], ["0px 8px", "0px 0px"], {
            ...clamp,
            easing: easeOut,
          }),
        }}
      >
        Used sextant: list_workflows
      </div>
      <div style={{ maxWidth: 900, fontWeight: 450 }}>
        {typed(ANSWER, frame, 58, 2.6)}
      </div>
    </div>
  );
};

export const Ask = () => (
  <Scene step={2} caption="Ask Claude what to automate" title="Claude">
    <Chat />
  </Scene>
);
