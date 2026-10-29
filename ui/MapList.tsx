import { useCallback, useEffect, useState } from "react";
import { ChevronRight } from "lucide-react";
import { api, type FirstMap, type MapRow } from "./api";
import { hours } from "./format";
import { t } from "./i18n";
import { Failed, List, Muted } from "./layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

const PAGE = 20;

function Row({ row, rank }: { row: MapRow; rank: number }) {
  return (
    <li className="flex gap-3 px-4 py-3">
      <span className="w-5 shrink-0 text-sm tabular-nums text-muted-foreground">{rank}</span>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="font-medium">{row.name}</span>
          {row.rough && <Badge variant="outline">{t("rough")}</Badge>}
        </div>
        <ol className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-sm text-muted-foreground">
          {row.steps.map((step, index) => (
            <li key={`${index}-${step}`} className="flex items-center gap-1.5">
              {index > 0 && <ChevronRight aria-hidden className="size-3.5 opacity-50" />}
              {step}
            </li>
          ))}
        </ol>
      </div>
      <div className="shrink-0 text-right">
        <div className="text-sm font-medium tabular-nums">{t("hoursWeek", { hours: hours(row.hours_week) })}</div>
        <div className="text-xs text-muted-foreground tabular-nums">{t("cases", { n: row.cases })}</div>
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

  if (error) return <Failed>{error}</Failed>;
  if (!map) return <Muted>{t("loading")}</Muted>;
  if (map.total === 0) return <Muted>{t("mapEmpty")}</Muted>;

  return (
    <div className="flex flex-col gap-3">
      {headline && (
        <p className="text-lg font-semibold tracking-tight">{t("mapHours", { hours: hours(map.hours_back) })}</p>
      )}
      <List>
        {rows.map((row, index) => (
          <Row key={row.id} row={row} rank={index + 1} />
        ))}
      </List>
      {rows.length < map.total && (
        <Button type="button" variant="outline" className="self-start" onClick={() => void load(rows.length)}>
          {t("showMore")}
        </Button>
      )}
    </div>
  );
}
