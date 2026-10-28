import { useCallback, useEffect, useState } from "react";
import { api, type Enjoyment, type Overview, type WorkflowRow } from "./api";
import { duration, hours } from "./format";
import { t } from "./i18n";
import WorkflowDetail from "./WorkflowDetail";

const PAGE = 20;
const ANSWERS: Enjoyment[] = ["hate", "dislike", "neutral", "like", "love"];

export function Steps({ steps }: { steps: WorkflowRow["steps"] }) {
  return (
    <ol className="steps">
      {steps.map((step) => (
        <li key={step.key}>
          {step.name}
          {step.gap_s !== null && (
            <small className="muted">
              {" "}
              {t("gapAfter", { time: duration(step.gap_s) })}
              {step.copies > 0 && `, ${t("copiedTimes", { n: step.copies })}`}
            </small>
          )}
        </li>
      ))}
    </ol>
  );
}

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
    <li className="workflow">
      <span className="rank">{rank}</span>
      <div>
        <div className="workflow-head">
          <strong>{row.name}</strong>
          <span className="tag">{t(`action_${row.action}`)}</span>
          {row.client && <span className="tag">{t("tagClient")}</span>}
          {row.handed_off && <span className="tag">{t("tagHandedOff")}</span>}
          {row.rough && <span className="tag">{t("rough")}</span>}
        </div>
        <Steps steps={row.steps} />
      </div>
      <div className="workflow-numbers">
        <span>{t("hoursWeek", { hours: hours(row.hours_week) })}</span>
        <span className="muted">{t("cases", { n: row.cases })}</span>
        <button type="button" className="link" onClick={() => onOpen(row.id)}>
          {t("open")}
        </button>
      </div>
    </li>
  );
}

function Ask({ rows, onAnswer }: { rows: WorkflowRow[]; onAnswer: (id: number, answer: Enjoyment) => void }) {
  if (rows.length === 0) return null;
  return (
    <section>
      <h2>{t("askTitle")}</h2>
      <p className="muted">{t("askBody")}</p>
      <ul className="workflows">
        {rows.map((row) => (
          <li key={row.id} className="ask">
            <strong>{row.name}</strong>
            <Steps steps={row.steps} />
            <div className="answers" role="group" aria-label={row.name}>
              {ANSWERS.map((answer) => (
                <button
                  key={answer}
                  type="button"
                  className="secondary"
                  onClick={() => onAnswer(row.id, answer)}
                >
                  {t(`enjoy_${answer}`)}
                </button>
              ))}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

function Section({
  title,
  body,
  rows,
  onOpen,
}: {
  title: string;
  body: string;
  rows: WorkflowRow[];
  onOpen: (id: number) => void;
}) {
  if (rows.length === 0) return null;
  return (
    <section>
      <h2>{title}</h2>
      <p className="muted">{body}</p>
      <ul className="workflows">
        {rows.map((row) => (
          <Row key={row.id} row={row} rank={null} onOpen={onOpen} />
        ))}
      </ul>
    </section>
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

  if (error) return <p role="alert">{error}</p>;
  if (!overview) return <p className="muted">{t("loading")}</p>;
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
  if (empty) return <p className="muted">{t("mapEmpty")}</p>;

  return (
    <>
      <p className="headline">{t("mapHours", { hours: hours(overview.hours_back) })}</p>
      <Ask rows={overview.ask} onAnswer={(id, enjoyment) => void answer(id, enjoyment)} />
      {ranked.length > 0 && (
        <section>
          <h2>{t("rankedTitle")}</h2>
          <ul className="workflows">
            {ranked.map((row, index) => (
              <Row key={row.id} row={row} rank={index + 1} onOpen={setOpen} />
            ))}
          </ul>
          {ranked.length < overview.total && (
            <button type="button" className="secondary" onClick={() => void load(ranked.length)}>
              {t("showMore")}
            </button>
          )}
        </section>
      )}
      <Section
        title={t("promisesTitle")}
        body={t("promisesBody")}
        rows={overview.promises}
        onOpen={setOpen}
      />
      <Section
        title={t("protectTitle")}
        body={t("protectBody")}
        rows={overview.protect}
        onOpen={setOpen}
      />
    </>
  );
}
