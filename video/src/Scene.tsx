import type React from "react";
import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";
import { clamp, color, easeOut, font } from "./theme";

/** A caption above a macOS-style window, the layout every scene shares. */
export const Scene = ({
  step,
  caption,
  title,
  children,
}: {
  readonly step: number;
  readonly caption: string;
  readonly title: string;
  readonly children: React.ReactNode;
}) => {
  const frame = useCurrentFrame();
  return (
    <AbsoluteFill
      style={{
        backgroundColor: color.background,
        fontFamily: font.sans,
        color: color.foreground,
        alignItems: "center",
        paddingTop: 56,
        gap: 36,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 20,
          fontSize: 48,
          fontWeight: 650,
          letterSpacing: -1,
          opacity: interpolate(frame, [0, 12], [0, 1], clamp),
          translate: interpolate(frame, [0, 12], ["0px 12px", "0px 0px"], {
            ...clamp,
            easing: easeOut,
          }),
        }}
      >
        <span
          style={{
            display: "grid",
            placeItems: "center",
            width: 52,
            height: 52,
            borderRadius: 26,
            backgroundColor: color.primary,
            color: "white",
            fontSize: 28,
            fontWeight: 700,
            letterSpacing: 0,
          }}
        >
          {step}
        </span>
        {caption}
      </div>
      <div
        style={{
          width: 1040,
          height: 470,
          borderRadius: 18,
          backgroundColor: color.card,
          boxShadow:
            "0 1px 2px oklch(0 0 0 / 6%), 0 24px 48px -16px oklch(0 0 0 / 18%)",
          outline: `1px solid ${color.border}`,
          overflow: "hidden",
          display: "flex",
          flexDirection: "column",
          opacity: interpolate(frame, [4, 16], [0, 1], clamp),
          scale: interpolate(frame, [4, 20], [0.97, 1], {
            ...clamp,
            easing: easeOut,
          }),
        }}
      >
        <div
          style={{
            position: "relative",
            display: "flex",
            alignItems: "center",
            height: 48,
            paddingLeft: 20,
            gap: 8,
            borderBottom: `1px solid ${color.border}`,
            backgroundColor: color.subtle,
          }}
        >
          {["#ff5f57", "#febc2e", "#28c840"].map((dot) => (
            <span
              key={dot}
              style={{
                width: 13,
                height: 13,
                borderRadius: 7,
                backgroundColor: dot,
              }}
            />
          ))}
          <span
            style={{
              position: "absolute",
              inset: 0,
              display: "grid",
              placeItems: "center",
              fontSize: 19,
              fontWeight: 600,
              color: color.muted,
            }}
          >
            {title}
          </span>
        </div>
        <div style={{ flex: 1, padding: "28px 40px" }}>{children}</div>
      </div>
    </AbsoluteFill>
  );
};
