import { useCallback, useEffect, useState } from "react";
import { api, type FirstMap, type MapRow } from "./api";
import { t } from "./i18n";

const PAGE = 20;

function hours(value: number): string {
  return value.toFixed(1);
}

function Row({ row, rank }: { row: MapRow; rank: number }) {
  return (
    <li className="workflow">
      <span className="rank">{rank}</span>
      <div className="workflow-main">
        <div className="workflow-head">
          <strong>{row.name}</strong>
          {row.rough && <span className="tag">{t("rough")}</span>}
        </div>
        <ol className="steps">
          {row.steps.map((step, index) => (
            <li key={`${index}-${step}`}>{step}</li>
          ))}
        </ol>
      </div>
      <div className="workflow-numbers">
        <span>{t("hoursWeek", { hours: hours(row.hours_week) })}</span>
        <span className="muted">{t("cases", { n: row.cases })}</span>
      </div>
    </li>
  );
}

/** The ranked workflows, loaded one page at a time. `headline` shows the hours a week. */
export default function MapList({ headline }: { headline: boolean }) {
  const [map, setMap] = useState<FirstMap | null>(null);
  const [rows, setRows] = useState<MapRow[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (offset: number) => {
    try {
      const page = await api.firstMap(offset, PAGE);
      setMap(page);
      setRows((current) => (offset === 0 ? page.rows : [...current, ...page.rows]));
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void load(0);
  }, [load]);

  if (error) return <p role="alert">{error}</p>;
  if (!map) return <p className="muted">{t("loading")}</p>;
  if (map.total === 0) return <p className="muted">{t("mapEmpty")}</p>;

  return (
    <>
      {headline && (
        <p className="headline">{t("mapHours", { hours: hours(map.hours_back) })}</p>
      )}
      <ul className="workflows">
        {rows.map((row, index) => (
          <Row key={row.id} row={row} rank={index + 1} />
        ))}
      </ul>
      {rows.length < map.total && (
        <button type="button" className="secondary" onClick={() => void load(rows.length)}>
          {t("showMore")}
        </button>
      )}
    </>
  );
}
