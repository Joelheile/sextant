import { interpolate, useCurrentFrame } from "remotion";
import { Scene } from "../Scene";
import { clamp, color, easeOut } from "../theme";

const MAX_MINUTES = 380;

const Row = ({
  label,
  minutes,
  index,
  pick = false,
}: {
  readonly label: string;
  readonly minutes: number;
  readonly index: number;
  readonly pick?: boolean;
}) => {
  const frame = useCurrentFrame();
  const start = 14 + index * 6;
  const picked = pick
    ? interpolate(frame, [62, 74], [0, 1], { ...clamp, easing: easeOut })
    : 0;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "1fr 150px",
        alignItems: "center",
        gap: 24,
        padding: "14px 20px",
        borderRadius: 12,
        backgroundColor: `color-mix(in oklch, ${color.primarySoft} ${picked * 100}%, transparent)`,
        opacity: interpolate(frame, [start, start + 8], [0, 1], clamp),
      }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 14,
            fontSize: 27,
            fontWeight: 560,
          }}
        >
          {label}
          {pick && (
            <span
              style={{
                fontSize: 19,
                fontWeight: 650,
                padding: "4px 12px",
                borderRadius: 99,
                backgroundColor: color.primary,
                color: "white",
                opacity: picked,
                scale: interpolate(picked, [0, 1], [0.9, 1]),
              }}
            >
              Automate: 73%
            </span>
          )}
        </div>
        <div
          style={{ height: 12, borderRadius: 6, backgroundColor: color.subtle }}
        >
          <div
            style={{
              height: "100%",
              borderRadius: 6,
              backgroundColor: pick
                ? color.primary
                : "oklch(0.216 0.006 56 / 22%)",
              width: `${interpolate(
                frame,
                [start, start + 26],
                [0, (minutes / MAX_MINUTES) * 100],
                {
                  ...clamp,
                  easing: easeOut,
                },
              )}%`,
            }}
          />
        </div>
      </div>
      <span
        style={{
          fontSize: 27,
          fontWeight: 600,
          textAlign: "right",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {hours > 0 ? `${hours} h ${rest} min` : `${rest} min`}
      </span>
    </div>
  );
};

export const Week = () => (
  <Scene step={1} caption="See where your week goes" title="Sextant: this week">
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <Row index={0} label="Meetings" minutes={370} />
      <Row index={1} label="Email" minutes={280} />
      <Row
        index={2}
        label="Invoices: Mail > Excel > lexoffice"
        minutes={90}
        pick
      />
      <Row index={3} label="Weekly report" minutes={70} />
    </div>
  </Scene>
);
