import { useCallback, useEffect, useState } from "react";
import { ChevronLeft } from "lucide-react";
import { api, type AutomationPlan, type Detail, type Enjoyment, type Understood } from "./api";
import { dateTime, hours } from "./format";
import { t } from "./i18n";
import { Done, Failed, Muted, Section } from "./layout";
import { Steps, Tags } from "./Workflows";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const BULLETS = "flex list-disc flex-col gap-1 pl-5 text-sm marker:text-muted-foreground";

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
    <Section title={t("correctTitle")} description={t("correctHint")}>
      <ul className="flex flex-col gap-2">
        {detail.workflow.steps.map((step) => (
          <li key={step.key}>
            <Label className="font-normal">
              <Checkbox checked={picked.includes(step.key)} onCheckedChange={() => toggle(step.key)} />
              {step.name}
            </Label>
          </li>
        ))}
      </ul>
      <Muted>{t("stepsMergeHint")}</Muted>
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void apply();
        }}
      >
        <Input
          type="text"
          value={phrase}
          placeholder={t("correctPlaceholder")}
          aria-label={t("correctTitle")}
          onChange={(e) => setPhrase(e.target.value)}
        />
        <Button type="submit" disabled={phrase.trim() === ""}>
          {t("correctApply")}
        </Button>
      </form>
      {understood === null && <Failed>{t("correctUnknown")}</Failed>}
      {understood && (
        <Done>
          {t(`understood_${understood.rule}`, {
            arg:
              understood.rule === "enjoyment" && understood.arg
                ? t(`enjoy_${understood.arg as Enjoyment}`)
                : (understood.arg ?? ""),
          })}
        </Done>
      )}
      {error && <Failed>{error}</Failed>}
    </Section>
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
    <Section title={t("automationTitle")} description={t("automationBody")}>
      {plan && (
        <Muted>
          {t("automationWhere")} <code className="font-mono text-xs break-all text-foreground">{plan.path}</code>
        </Muted>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="button" disabled={!plan} onClick={() => void create()}>
          {t("automationCreate")}
        </Button>
        <Button type="button" variant="outline" onClick={() => void choose()}>
          {t("automationChoose")}
        </Button>
        <Button
          type="button"
          variant="ghost"
          disabled={detail.workflow.handed_off || handedOff}
          onClick={() => void handoff()}
        >
          {t("handoffButton")}
        </Button>
      </div>
      {done && <Done>{t("automationDone", { path: done })}</Done>}
      {handedOff && <Done>{t("handoffDone")}</Done>}
      {error && <Failed>{error}</Failed>}
    </Section>
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
    <Button type="button" variant="ghost" size="sm" className="-ml-2 self-start text-muted-foreground" onClick={onBack}>
      <ChevronLeft data-icon="inline-start" />
      {t("back")}
    </Button>
  );
  if (error) return <Failed>{error}</Failed>;
  if (gone) {
    return (
      <>
        {back}
        <Muted>{t("workflowGone")}</Muted>
      </>
    );
  }
  if (!detail) return <Muted>{t("loading")}</Muted>;
  const { workflow } = detail;

  return (
    <>
      <header className="flex flex-col gap-2">
        {back}
        <h1 className="text-2xl font-semibold tracking-tight">{workflow.name}</h1>
        <div className="flex flex-wrap items-center gap-1.5 text-sm text-muted-foreground tabular-nums">
          <Tags row={workflow} />
          <span className="ml-1">
            {t("hoursWeek", { hours: hours(workflow.hours_week) })}, {t("cases", { n: workflow.cases })}
          </span>
        </div>
        <Steps steps={workflow.steps} />
      </header>

      <Section title={t("reasonsTitle")}>
        <ul className={BULLETS}>
          {detail.reasons.map((reason) => (
            <li key={reason}>{reason}</li>
          ))}
        </ul>
      </Section>

      {detail.copy_points.length > 0 && (
        <Section title={t("copyPointsTitle")}>
          <ul className={BULLETS}>
            {detail.copy_points.map((point) => (
              <li key={`${point.from}-${point.to}`}>
                {t("copyPoint", { from: point.from, to: point.to, n: point.copies })}
              </li>
            ))}
          </ul>
        </Section>
      )}

      {detail.examples.length > 0 && (
        <Section title={t("examplesTitle")} description={t("examplesNote")}>
          <ul className="flex flex-col gap-2 text-sm">
            {detail.examples.map((example) => (
              <li key={`${example.entity}-${example.first_ts}`}>
                <span className="font-medium">{example.entity}</span>{" "}
                <span className="text-muted-foreground tabular-nums">{dateTime(example.first_ts)}</span>
                <div className="text-muted-foreground">{example.path.join(" › ")}</div>
              </li>
            ))}
          </ul>
        </Section>
      )}

      {detail.commitments.length > 0 && (
        <Section title={t("commitmentsTitle")}>
          <ul className={BULLETS}>
            {detail.commitments.map((c) => (
              <li key={`${c.ts}-${c.text}`}>
                {c.text} <span className="text-muted-foreground tabular-nums">{dateTime(c.ts)}</span>
                {c.overdue && (
                  <Badge variant="destructive" className="ml-1.5">
                    {t("overdue")}
                  </Badge>
                )}
              </li>
            ))}
          </ul>
        </Section>
      )}

      <Automation detail={detail} onHandedOff={() => void load()} />
      <Correction detail={detail} onChanged={() => void load()} />
    </>
  );
}
