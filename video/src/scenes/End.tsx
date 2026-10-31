import {
  AbsoluteFill,
  Img,
  interpolate,
  staticFile,
  useCurrentFrame,
} from "remotion";
import { clamp, color, easeOut, font } from "../theme";

export const End = () => {
  const frame = useCurrentFrame();
  return (
    <AbsoluteFill
      style={{
        backgroundColor: color.background,
        fontFamily: font.sans,
        color: color.foreground,
        alignItems: "center",
        justifyContent: "center",
        gap: 22,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 24,
          opacity: interpolate(frame, [0, 12], [0, 1], clamp),
          scale: interpolate(frame, [0, 18], [0.96, 1], {
            ...clamp,
            easing: easeOut,
          }),
        }}
      >
        <Img src={staticFile("icon.png")} style={{ width: 104, height: 104 }} />
        <span style={{ fontSize: 96, fontWeight: 700, letterSpacing: -3 }}>
          Sextant
        </span>
      </div>
      <div
        style={{
          fontSize: 44,
          fontWeight: 560,
          opacity: interpolate(frame, [8, 20], [0, 1], clamp),
        }}
      >
        Find the work worth automating.
      </div>
      <div
        style={{
          fontSize: 28,
          color: color.muted,
          opacity: interpolate(frame, [16, 28], [0, 1], clamp),
        }}
      >
        Text only. No screenshots, no audio, no telemetry.
      </div>
    </AbsoluteFill>
  );
};
