import { Easing } from "remotion";

/** The app's light palette from ui/styles.css. */
export const color = {
  background: "oklch(0.97 0.002 106)",
  card: "oklch(1 0 0)",
  foreground: "oklch(0.216 0.006 56)",
  muted: "oklch(0.553 0.013 58)",
  subtle: "oklch(0.95 0.002 106)",
  border: "oklch(0.923 0.003 49)",
  primary: "oklch(0.511 0.096 186.4)",
  primarySoft: "oklch(0.95 0.03 186.4)",
};

export const font = {
  sans: '-apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", sans-serif',
  mono: 'ui-monospace, "SF Mono", Menlo, monospace',
};

export const easeOut = Easing.bezier(0.23, 1, 0.32, 1);

export const clamp = {
  extrapolateLeft: "clamp",
  extrapolateRight: "clamp",
} as const;
