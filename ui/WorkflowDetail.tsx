import { useCallback, useEffect, useState } from "react";
import { api, type AutomationPlan, type Detail, type Enjoyment, type Understood } from "./api";
import { dateTime, hours } from "./format";
import { t } from "./i18n";
import { Steps } from "./Workflows";

function Correction({
  detail,
  onChanged,
}: {
  detail: Detail;
  onChanged: () => void;
}) {
  const [phrase, setPhrase] = useState("");
  const [picked, setPicked] = useState<string[]>([]);
  const [understood, setUnderstood] = useState<Understood | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);

  const toggle = (key: string) =>
    setPicked((current) =>
      current.includes(key) ? current.filter((k) => k !== key) : [...current, key],
    );

  const apply = async () => {
    try {
      const result = await api.correctWorkflow(detail.workflow.id, phrase, picked);
      setUnderstood(result);
      setError(null);
      if (result) {
        setPhrase("");
        setPicked([]);
        onChanged();
      }
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <section>
      <h2>{t("correctTitle")}</h2>
      <p className="muted">{t("correctHint")}</p>
      <ul className="picks">
        {detail.workflow.steps.map((step) => (
          <li key={step.key}>
            <label className="switch">
              <input
                type="checkbox"
                checked={picked.includes(step.key)}
                onChange={() => toggle(step.key)}
              />
              {step.name}
            </label>
          </li>
        ))}
      </ul>
      <p className="muted">{t("stepsMergeHint")}</p>
      <form
        className="inline"
        onSubmit={(e) => {
          e.preventDefault();
          void apply();
        }}
      >
        <input
          type="text"
          value={phrase}
          placeholder={t("correctPlaceholder")}
          aria-label={t("correctTitle")}
          onChange={(e) => setPhrase(e.target.value)}
        />
        <button type="submit" disabled={phrase.trim() === ""}>
          {t("correctApply")}
        </button>
      </form>
      {understood === null && <p role="alert">{t("correctUnknown")}</p>}
      {understood && (
        <p className="ok" aria-live="polite">
          {t(`understood_${understood.rule}`, {
            arg:
              understood.rule === "enjoyment" && understood.arg
                ? t(`enjoy_${understood.arg as Enjoyment}`)
                : (understood.arg ?? ""),
          })}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

function Automation({ detail, onHandedOff }: { detail: Detail; onHandedOff: () => void }) {
  const id = detail.workflow.id;
  const [folder, setFolder] = useState<string | null>(null);
  const [plan, setPlan] = useState<AutomationPlan | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [handedOff, setHandedOff] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .automationPlan(id, folder)
      .then(setPlan)
      .catch((err) => setError(String(err)));
  }, [id, folder]);

  const choose = async () => {
    try {
      const picked = await api.pickFolder(t("automationFolderPrompt"));
      if (picked) {
        setFolder(picked);
        setDone(null);
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const create = async () => {
    if (!plan) return;
    setError(null);
    try {
      // The copy starts inside the click, which the clipboard needs.
      const copied = navigator.clipboard.writeText(plan.prompt);
      const path = await api.writeSkill(id, folder);
      await copied;
      setDone(path);
    } catch (err) {
      setError(String(err));
    }
  };

  const handoff = async () => {
    try {
      await api.reportHandoff(id);
      setHandedOff(true);
      onHandedOff();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <section>
      <h2>{t("automationTitle")}</h2>
      <p className="muted">{t("automationBody")}</p>
      {plan && (
        <p className="muted">
          {t("automationWhere")} <code>{plan.path}</code>
        </p>
      )}
      <div className="actions-row">
        <button type="button" className="secondary" onClick={() => void choose()}>
          {t("automationChoose")}
        </button>
        <button type="button" disabled={!plan} onClick={() => void create()}>
          {t("automationCreate")}
        </button>
        <button
          type="button"
          className="secondary"
          disabled={detail.workflow.handed_off || handedOff}
          onClick={() => void handoff()}
        >
          {t("handoffButton")}
        </button>
      </div>
      {done && (
        <p className="ok" aria-live="polite">
          {t("automationDone", { path: done })}
        </p>
      )}
      {handedOff && (
        <p className="ok" aria-live="polite">
          {t("handoffDone")}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

/** One workflow: reasons for each score, redacted examples, corrections and the handoff. */
export default function WorkflowDetail({ id, onBack }: { id: number; onBack: () => void }) {
  const [detail, setDetail] = useState<Detail | null>(null);
  const [gone, setGone] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(
    () =>
      api
        .workflowDetail(id)
        .then((found) => {
          setDetail(found);
          setGone(found === null);
        })
        .catch((err) => setError(String(err))),
    [id],
  );

  useEffect(() => {
    void load();
  }, [load]);

  const back = (
    <button type="button" className="link" onClick={onBack}>
      {t("back")}
    </button>
  );
  if (error) return <p role="alert">{error}</p>;
  if (gone) {
    return (
      <>
        {back}
        <p className="muted">{t("workflowGone")}</p>
      </>
    );
  }
  if (!detail) return <p className="muted">{t("loading")}</p>;
  const { workflow } = detail;

  return (
    <>
      {back}
      <h1>{workflow.name}</h1>
      <p>
        <span className="tag">{t(`action_${workflow.action}`)}</span>{" "}
        <span className="muted">
          {t("hoursWeek", { hours: hours(workflow.hours_week) })}, {t("cases", { n: workflow.cases })}
        </span>
      </p>
      <Steps steps={workflow.steps} />

      <section>
        <h2>{t("reasonsTitle")}</h2>
        <ul>
          {detail.reasons.map((reason) => (
            <li key={reason}>{reason}</li>
          ))}
        </ul>
      </section>

      {detail.copy_points.length > 0 && (
        <section>
          <h2>{t("copyPointsTitle")}</h2>
          <ul>
            {detail.copy_points.map((point) => (
              <li key={`${point.from}-${point.to}`}>
                {t("copyPoint", { from: point.from, to: point.to, n: point.copies })}
              </li>
            ))}
          </ul>
        </section>
      )}

      {detail.examples.length > 0 && (
        <section>
          <h2>{t("examplesTitle")}</h2>
          <p className="muted">{t("examplesNote")}</p>
          <ul className="examples">
            {detail.examples.map((example) => (
              <li key={`${example.entity}-${example.first_ts}`}>
                <strong>{example.entity}</strong>{" "}
                <span className="muted">{dateTime(example.first_ts)}</span>
                <div className="muted">{example.path.join(" › ")}</div>
              </li>
            ))}
          </ul>
        </section>
      )}

      {detail.commitments.length > 0 && (
        <section>
          <h2>{t("commitmentsTitle")}</h2>
          <ul>
            {detail.commitments.map((c) => (
              <li key={`${c.ts}-${c.text}`}>
                {c.text} <span className="muted">{dateTime(c.ts)}</span>
                {c.overdue && <span className="tag">{t("overdue")}</span>}
              </li>
            ))}
          </ul>
        </section>
      )}

      <Correction detail={detail} onChanged={() => void load()} />
      <Automation detail={detail} onHandedOff={() => void load()} />
    </>
  );
}
