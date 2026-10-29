import { useEffect, useState } from "react";
import { api, type ResultRow, type Results as ResultsData } from "./api";
import { hours, shortDate } from "./format";
import { t } from "./i18n";
import { Failed, List, Muted, Section } from "./layout";
import Report from "./Report";
import { Progress } from "@/components/ui/progress";

function Row({ row, minDays }: { row: ResultRow; minDays: number }) {
  const measured = row.hours_before !== null && row.hours_after !== null;
  return (
    <li className="flex flex-col gap-2 px-4 py-3">
      <div className="flex items-baseline justify-between gap-3">
        <span className="font-medium">{row.name}</span>
        <span className="text-xs text-muted-foreground tabular-nums">
          {t("handoffOn", { date: shortDate(row.ts) })}
        </span>
      </div>
      {measured ? (
        <div className="flex flex-wrap items-baseline gap-x-5 gap-y-1 text-sm tabular-nums">
          <span>
            <span className="text-muted-foreground">{t("hoursBefore")}</span>{" "}
            {t("hoursWeek", { hours: hours(row.hours_before ?? 0) })}
          </span>
          <span>
            <span className="text-muted-foreground">{t("hoursAfter")}</span>{" "}
            {t("hoursWeek", { hours: hours(row.hours_after ?? 0) })}
          </span>
          <span className="font-medium text-success">{t("hoursBack", { hours: hours(row.saved ?? 0) })}</span>
        </div>
      ) : (
        <div aria-live="polite" className="flex items-center gap-3">
          <Progress value={(row.progress / minDays) * 100} className="max-w-48" />
          <span className="text-xs text-muted-foreground tabular-nums">
            {t("measuring", { n: row.progress, total: minDays })}
          </span>
        </div>
      )}
    </li>
  );
}

/** Hours a week before and after each handoff, and the weekly report. */
export default function Results() {
  const [results, setResults] = useState<ResultsData | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .results()
      .then(setResults)
      .catch((err) => setError(String(err)));
  }, []);

  if (error) return <Failed>{error}</Failed>;
  if (!results) return <Muted>{t("loading")}</Muted>;

  return (
    <>
      <Section title={t("resultsTitle")}>
        {results.rows.length === 0 ? (
          <Muted>{t("resultsEmpty")}</Muted>
        ) : (
          <List>
            {results.rows.map((row) => (
              <Row key={row.handoff_id} row={row} minDays={results.min_days} />
            ))}
          </List>
        )}
      </Section>
      <Report />
    </>
  );
}
