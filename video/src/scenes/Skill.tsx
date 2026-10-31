import type { CSSProperties } from "react";
import { interpolate, useCurrentFrame } from "remotion";
import { Scene } from "../Scene";
import { clamp, color, font } from "../theme";

type Kind = "meta" | "heading" | "text";

const LINES: [Kind, string][] = [
  ["meta", "---"],
  ["meta", "name: invoices-mail-excel-lexoffice"],
  ["meta", "description: Enter an emailed invoice in lexoffice."],
  ["meta", "---"],
  ["heading", "# Invoices: Mail > Excel > lexoffice"],
  ["text", ""],
  ["heading", "## Steps"],
  ["text", "1. Mail: open the invoice attachment"],
  ["text", "2. Excel: add the number and amount to invoices.xlsx"],
  ["text", "3. lexoffice: create the voucher with the same values"],
  ["text", ""],
  ["text", "Show the user what you will do before you pay anything."],
];

const STYLE: Record<Kind, CSSProperties> = {
  meta: { color: color.muted },
  heading: { color: color.primary, fontWeight: 700 },
  text: {},
};

const File = () => {
  const frame = useCurrentFrame();
  return (
    <div style={{ fontFamily: font.mono, fontSize: 22, lineHeight: 1.5 }}>
      {LINES.map(([kind, text], index) => (
        <div
          key={index}
          style={{
            ...STYLE[kind],
            minHeight: 33,
            whiteSpace: "pre",
            opacity: interpolate(
              frame,
              [12 + index * 4, 16 + index * 4],
              [0, 1],
              clamp,
            ),
          }}
        >
          {text}
        </div>
      ))}
    </div>
  );
};

export const Skill = () => (
  <Scene
    step={3}
    caption="Get a skill Claude can run"
    title="invoices/SKILL.md"
  >
    <File />
  </Scene>
);
