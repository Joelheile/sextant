import { useCallback, useEffect, useState } from "react";
import { ChevronRight } from "lucide-react";
import { api, type Enjoyment, type Overview, type WorkflowRow } from "./api";
import { duration, hours } from "./format";
import { t } from "./i18n";
import { Failed, List, Muted, Section } from "./layout";
import WorkflowDetail from "./WorkflowDetail";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

const PAGE = 20;
const ANSWERS: Enjoyment[] = ["hate", "dislike", "neutral", "like", "love"];

export function Steps({ steps }: { steps: WorkflowRow["steps"] }) {
  return (
    <ol className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-sm text-muted-foreground">
      {steps.map((step, index) => (
        <li key={step.key} className="flex items-center gap-1.5">
          {index > 0 && <ChevronRight aria-hidden className="size-3.5 opacity-50" />}
          {step.name}
          {step.gap_s !== null && (
            <small className="text-xs opacity-75">
              {t("gapAfter", { time: duration(step.gap_s) })}
              {step.copies > 0 && `, ${t("copiedTimes", { n: step.copies })}`}
            </small>
          )}
        </li>
      ))}
    </ol>
  );
}

export function Tags({ row }: { row: WorkflowRow }) {
  return (
    <>
      <Badge variant="secondary">{t(`action_${row.action}`)}</Badge>
      {row.client && <Badge variant="outline">{t("tagClient")}</Badge>}
      {row.handed_off && <Badge variant="outline">{t("tagHandedOff")}</Badge>}
      {row.rough && <Badge variant="outline">{t("rough")}</Badge>}
    </>
  );
}

/** The whole row opens the workflow; the name is the button. */
function Row({
  row,
  rank,
  onOpen,
}: {
  row: WorkflowRow;
  rank: number | null;
  onOpen: (id: number) => void;
}) {
  return (
    <li className="relative flex gap-3 px-4 py-3 transition-colors hover:bg-muted/60 has-focus-visible:bg-muted/60">
      {rank !== null && <span className="w-5 shrink-0 text-sm tabular-nums text-muted-foreground">{rank}</span>}
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-1.5">
          <button
            type="button"
            className="text-left font-medium outline-none after:absolute after:inset-0"
            onClick={() => onOpen(row.id)}
          >
            {row.name}
          </button>
          <Tags row={row} />
        </div>
        <Steps steps={row.steps} />
      </div>
      <div className="shrink-0 text-right">
        <div className="text-sm font-medium tabular-nums">{t("hoursWeek", { hours: hours(row.hours_week) })}</div>
        <div className="text-xs text-muted-foreground tabular-nums">{t("cases", { n: row.cases })}</div>
      </div>
    </li>
  );
}

function Ask({ rows, onAnswer }: { rows: WorkflowRow[]; onAnswer: (id: number, answer: Enjoyment) => void }) {
  if (rows.length === 0) return null;
  return (
    <Section title={t("askTitle")} description={t("askBody")}>
      <List>
        {rows.map((row) => (
          <li key={row.id} className="flex flex-col gap-2 px-4 py-3">
            <span className="font-medium">{row.name}</span>
            <Steps steps={row.steps} />
            <div className="flex flex-wrap gap-1.5 pt-1" role="group" aria-label={row.name}>
              {ANSWERS.map((answer) => (
                <Button key={answer} type="button" variant="outline" size="sm" onClick={() => onAnswer(row.id, answer)}>
                  {t(`enjoy_${answer}`)}
                </Button>
              ))}
            </div>
          </li>
        ))}
      </List>
    </Section>
  );
}

function Rows({
  title,
  description,
  rows,
  ranked,
  onOpen,
}: {
  title: string;
  description?: string;
  rows: WorkflowRow[];
  ranked?: boolean;
  onOpen: (id: number) => void;
}) {
  if (rows.length === 0) return null;
  return (
    <Section title={title} description={description}>
      <List>
        {rows.map((row, index) => (
          <Row key={row.id} row={row} rank={ranked ? index + 1 : null} onOpen={onOpen} />
        ))}
      </List>
    </Section>
  );
}

/** The default screen: hours to get back, the questions, the ranked workflows. */
export default function Workflows() {
  const [overview, setOverview] = useState<Overview | null>(null);
  const [ranked, setRanked] = useState<WorkflowRow[]>([]);
  const [open, setOpen] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (offset: number) => {
    try {
      const page = await api.workflowOverview(offset, PAGE);
      setOverview(page);
      setRanked((current) => (offset === 0 ? page.ranked : [...current, ...page.ranked]));
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void load(0);
  }, [load]);

  const answer = async (id: number, enjoyment: Enjoyment) => {
    try {
      await api.answerEnjoyment(id, enjoyment);
      await load(0);
    } catch (err) {
      setError(String(err));
    }
  };

  if (error) return <Failed>{error}</Failed>;
  if (!overview) return <Muted>{t("loading")}</Muted>;
  if (open !== null) {
    return (
      <WorkflowDetail
        id={open}
        onBack={() => {
          setOpen(null);
          void load(0);
        }}
      />
    );
  }
  const empty =
    overview.total === 0 && overview.promises.length === 0 && overview.protect.length === 0;
  if (empty) return <Muted>{t("mapEmpty")}</Muted>;

  return (
    <>
      <h1 className="text-2xl font-semibold tracking-tight">
        {t("mapHours", { hours: hours(overview.hours_back) })}
      </h1>
      <Ask rows={overview.ask} onAnswer={(id, enjoyment) => void answer(id, enjoyment)} />
      {ranked.length > 0 && (
        <div className="flex flex-col gap-3">
          <Rows title={t("rankedTitle")} rows={ranked} ranked onOpen={setOpen} />
          {ranked.length < overview.total && (
            <Button type="button" variant="outline" className="self-start" onClick={() => void load(ranked.length)}>
              {t("showMore")}
            </Button>
          )}
        </div>
      )}
      <Rows title={t("promisesTitle")} description={t("promisesBody")} rows={overview.promises} onOpen={setOpen} />
      <Rows title={t("protectTitle")} description={t("protectBody")} rows={overview.protect} onOpen={setOpen} />
    </>
  );
}
