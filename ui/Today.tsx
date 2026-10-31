import { useEffect, useState } from "react";
import { api, type Insights } from "./api";
import { duration } from "./format";
import { t, type Key } from "./i18n";
import { Failed, Muted, Section } from "./layout";

const WEEKDAY = new Intl.DateTimeFormat(navigator.language, {
  weekday: "narrow",
});

/** `312` becomes "5 h 12 min", so a day's total keeps its minutes. */
function minutes(value: number): string {
  const total = Math.round(value);
  const rest = total % 60;
  if (total < 60) return t("durMinutes", { n: total });
  if (rest === 0) return duration(total * 60);
  return `${t("durHours", { n: Math.floor(total / 60) })} ${t("durMinutes", { n: rest })}`;
}

/** Seven bars, today last and highlighted. */
function Week({ days }: { days: number[] }) {
  const max = Math.max(...days, 1);
  const today = new Date();
  return (
    <ol aria-label={t("weekLabel")} className="flex h-14 items-end gap-1.5">
      {days.map((value, i) => {
        const day = new Date(today);
        day.setDate(today.getDate() - (days.length - 1 - i));
        const last = i === days.length - 1;
        return (
          <li
            key={i}
            className="flex flex-1 flex-col items-center gap-1"
            title={minutes(value)}
          >
            <span
              className={
                last
                  ? "w-full rounded-sm bg-primary"
                  : "w-full rounded-sm bg-foreground/15"
              }
              style={{ height: `${Math.max(2, (value / max) * 40)}px` }}
            />
            <span className="text-[10px] text-muted-foreground">
              {WEEKDAY.format(day)}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

function Stat({ label, value }: { label: Key; value: string }) {
  return (
    <div className="flex flex-col gap-0.5 rounded-lg bg-foreground/[0.04] px-3 py-2 ring-1 ring-foreground/5">
      <span className="text-lg font-semibold tabular-nums">{value}</span>
      <span className="text-xs text-muted-foreground">{t(label)}</span>
    </div>
  );
}

/** The Today tab: time at the Mac, apps, focus and what was captured. Computed on this Mac. */
export default function Today({ opened }: { opened: number }) {
  const [insights, setInsights] = useState<Insights | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .insights()
      .then(setInsights)
      .catch((err) => setError(String(err)));
  }, [opened]);

  if (error) return <Failed>{error}</Failed>;
  if (!insights) return <Muted>{t("loading")}</Muted>;

  const top = insights.apps[0]?.minutes ?? 1;
  return (
    <>
      <div className="flex items-end justify-between gap-4">
        <div className="flex flex-col">
          <span className="text-3xl font-semibold tracking-tight tabular-nums">
            {minutes(insights.active_minutes)}
          </span>
          <span className="text-sm text-muted-foreground">
            {t("activeToday")}
          </span>
        </div>
        <div className="w-40">
          <Week days={insights.days} />
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2">
        <Stat
          label="statLongestFocus"
          value={minutes(insights.longest_focus_minutes)}
        />
        <Stat
          label="statSwitches"
          value={
            insights.switches_per_hour === null
              ? "–"
              : insights.switches_per_hour.toFixed(0)
          }
        />
        <Stat
          label="statLines"
          value={insights.lines.toLocaleString(navigator.language)}
        />
        <Stat label="statCalls" value={minutes(insights.call_minutes)} />
      </div>

      <Section title={t("topApps")}>
        {insights.apps.length === 0 ? (
          <Muted>{t("noActivity")}</Muted>
        ) : (
          <ul className="flex flex-col gap-2">
            {insights.apps.map(({ app, minutes: spent }) => (
              <li key={app} className="flex flex-col gap-1">
                <div className="flex justify-between text-sm">
                  <span className="truncate">{app}</span>
                  <span className="text-muted-foreground tabular-nums">
                    {minutes(spent)}
                  </span>
                </div>
                <span className="h-1 rounded-full bg-foreground/10">
                  <span
                    className="block h-full rounded-full bg-primary/70"
                    style={{ width: `${(spent / top) * 100}%` }}
                  />
                </span>
              </li>
            ))}
          </ul>
        )}
      </Section>
      <Muted className="text-xs">{t("insightsNote")}</Muted>
    </>
  );
}
