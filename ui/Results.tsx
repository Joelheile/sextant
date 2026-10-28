import { useEffect, useState } from "react";
import { api, type ResultRow, type Results as ResultsData } from "./api";
import { hours, shortDate } from "./format";
import { t } from "./i18n";
import Report from "./Report";

function Row({ row, minDays }: { row: ResultRow; minDays: number }) {
  const measured = row.hours_before !== null && row.hours_after !== null;
  return (
    <li className="result">
      <div className="workflow-head">
        <strong>{row.name}</strong>
        <span className="muted">{t("handoffOn", { date: shortDate(row.ts) })}</span>
      </div>
      {measured ? (
        <div className="before-after">
          <span>
            <span className="muted">{t("hoursBefore")}</span>{" "}
            {t("hoursWeek", { hours: hours(row.hours_before ?? 0) })}
          </span>
          <span>
            <span className="muted">{t("hoursAfter")}</span>{" "}
            {t("hoursWeek", { hours: hours(row.hours_after ?? 0) })}
          </span>
          <strong>{t("hoursBack", { hours: hours(row.saved ?? 0) })}</strong>
        </div>
      ) : (
        <div aria-live="polite">
          <progress value={row.progress} max={minDays} />{" "}
          <span className="muted">{t("measuring", { n: row.progress, total: minDays })}</span>
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

  if (error) return <p role="alert">{error}</p>;
  if (!results) return <p className="muted">{t("loading")}</p>;

  return (
    <>
      <h2>{t("resultsTitle")}</h2>
      {results.rows.length === 0 ? (
        <p className="muted">{t("resultsEmpty")}</p>
      ) : (
        <ul className="workflows">
          {results.rows.map((row) => (
            <Row key={row.handoff_id} row={row} minDays={results.min_days} />
          ))}
        </ul>
      )}
      <Report />
    </>
  );
}
