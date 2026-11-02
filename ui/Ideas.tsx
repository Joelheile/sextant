import { useEffect, useId, useState } from "react";
import { api, type AgentName, type AgentStatus, type Insights } from "./api";
import { dateTime, hours } from "./format";
import { Check, Copy, LoaderCircle, Sparkles, TrendingDown, TrendingUp } from "lucide-react";
import { t } from "./i18n";
import { Failed, List, Muted, Section } from "./layout";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";

const AGENT_LABEL: Record<AgentName, string> = {
  claude: "Claude Code",
  codex: "Codex",
};

/** The line that has your agent turn a routine into a skill. Claude Code runs MCP prompts as
 * slash commands; Codex gets the request in words and finds check_skill on its own. */
function skillPrompt(agent: AgentName, id: number): string {
  return agent === "claude"
    ? `/mcp__sextant__write_skill ${id}`
    : `Write a skill for Sextant routine ${id}.`;
}

function SkillPrompt({ agent, id }: { agent: AgentName; id: number }) {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const line = skillPrompt(agent, id);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(line);
      setCopied(true);
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div className="flex flex-col gap-1 pt-1">
      <span className="text-xs text-muted-foreground">
        {t("routineSkill", { agent: AGENT_LABEL[agent] })}
      </span>
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded-md bg-muted px-2 py-1 font-mono text-xs">
          {line}
        </code>
        <Button size="sm" variant="outline" onClick={() => void copy()}>
          {copied ? <Check /> : <Copy />}
          {copied ? t("copied") : t("copy")}
        </Button>
      </div>
      {error && <Failed>{error}</Failed>}
    </div>
  );
}

/** Hours a week to get back, and how this week's automatable time compares with last week. */
function Potential({ insights }: { insights: Insights }) {
  const now = insights.automatable_hours;
  const before = insights.automatable_hours_last_week;
  const change =
    before > 0 ? Math.round(((now - before) / before) * 100) : null;
  return (
    <div className="flex flex-col gap-1">
      <span className="text-3xl font-semibold tracking-tight tabular-nums">
        {t("hoursWeek", { hours: hours(insights.hours_back) })}
      </span>
      <span className="text-sm text-muted-foreground">{t("couldGetBack")}</span>
      <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
        {change !== null &&
          (change >= 0 ? (
            <TrendingUp className="size-3.5" />
          ) : (
            <TrendingDown className="size-3.5" />
          ))}
        {change === null
          ? t("automatableWeek", { hours: hours(now) })
          : t("automatableTrend", {
              hours: hours(now),
              change: `${change >= 0 ? "+" : ""}${change}`,
            })}
      </span>
    </div>
  );
}

/** The Ideas tab: automation potential and the routines your own agent found. */
export default function Ideas({ opened }: { opened: number }) {
  const [insights, setInsights] = useState<Insights | null>(null);
  const [agent, setAgent] = useState<AgentStatus | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.insights(), api.agentStatus()])
      .then(([found, status]) => {
        setInsights(found);
        setAgent(status);
        setRunning(status.running);
      })
      .catch((err) => setError(String(err)));
  }, [opened]);

  const analyze = async () => {
    setRunning(true);
    setError(null);
    try {
      setAgent(await api.runAgent());
    } catch (err) {
      setError(String(err));
    } finally {
      setRunning(false);
    }
  };

  if (!insights || !agent)
    return error ? <Failed>{error}</Failed> : <Muted>{t("loading")}</Muted>;

  return (
    <>
      <Potential insights={insights} />

      <Section
        title={t("routinesTitle")}
        description={
          agent.last_run && agent.last_agent
            ? t("routinesFrom", {
                agent: AGENT_LABEL[agent.last_agent],
                time: dateTime(agent.last_run),
              })
            : t("routinesNone")
        }
      >
        {agent.chosen ? (
          <Button
            className="self-start"
            disabled={running}
            onClick={() => void analyze()}
          >
            {running ? <LoaderCircle className="animate-spin" /> : <Sparkles />}
            {running
              ? t("analyzing")
              : t("analyzeWith", { agent: AGENT_LABEL[agent.chosen] })}
          </Button>
        ) : (
          <Muted>{t("pickAgent")}</Muted>
        )}
        {error && <Failed>{error}</Failed>}
        {agent.routines.length > 0 && (
          <List>
            {agent.routines.map((routine) => (
              <li key={routine.id} className="flex flex-col gap-1 px-4 py-3">
                <div className="flex items-baseline justify-between gap-3">
                  <span className="text-sm font-medium">{routine.title}</span>
                  <span className="shrink-0 text-sm text-primary tabular-nums">
                    {t("hoursWeek", { hours: hours(routine.hours_week) })}
                  </span>
                </div>
                <p className="text-sm text-muted-foreground">{routine.how}</p>
                {routine.approval && (
                  <p className="text-sm text-muted-foreground">
                    {t("routineStop", { action: routine.approval })}
                  </p>
                )}
                {routine.first && routine.last && (
                  <span className="text-xs text-muted-foreground">
                    {t("routineSeen", {
                      count: routine.activities,
                      first: routine.first.slice(0, 10),
                      last: routine.last.slice(0, 10),
                    })}
                  </span>
                )}
                <SkillPrompt agent={agent.chosen ?? "claude"} id={routine.id} />
              </li>
            ))}
          </List>
        )}
      </Section>
    </>
  );
}

/** Settings: which of your own agents the bridge runs, and whether weekly. */
export function AgentBridge() {
  const [status, setStatus] = useState<AgentStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const weeklyId = useId();

  useEffect(() => {
    api
      .agentStatus()
      .then(setStatus)
      .catch((err) => setError(String(err)));
  }, []);

  const save = (agent: AgentName | null, weekly: boolean) =>
    api
      .setAgent(agent, weekly)
      .then((settings) =>
        setStatus(
          (current) =>
            current && {
              ...current,
              chosen: settings.agent,
              weekly: settings.agent_weekly,
            },
        ),
      )
      .catch((err) => setError(String(err)));

  if (!status) return error ? <Failed>{error}</Failed> : null;

  return (
    <Section title={t("bridgeTitle")} description={t("bridgeBody")}>
      <List>
        <li className="flex items-center justify-between gap-4 px-4 py-3">
          <span className="text-sm">{t("bridgeAgent")}</span>
          <Select
            value={status.chosen ?? "off"}
            onValueChange={(value) =>
              void save(
                value === "off" ? null : (value as AgentName),
                status.weekly,
              )
            }
          >
            <SelectTrigger aria-label={t("bridgeAgent")} className="min-w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="off">{t("bridgeOff")}</SelectItem>
              {(Object.keys(AGENT_LABEL) as AgentName[]).map((name) => (
                <SelectItem
                  key={name}
                  value={name}
                  disabled={!status.installed.includes(name)}
                >
                  {status.installed.includes(name)
                    ? AGENT_LABEL[name]
                    : t("notInstalled", { agent: AGENT_LABEL[name] })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </li>
        <li className="flex items-center justify-between gap-4 px-4 py-3">
          <Label htmlFor={weeklyId} className="font-normal leading-snug">
            {t("bridgeWeekly")}
          </Label>
          <Switch
            id={weeklyId}
            checked={status.weekly}
            disabled={!status.chosen}
            onCheckedChange={(on) => void save(status.chosen, on)}
          />
        </li>
      </List>
      <Muted className="text-xs">{t("bridgeSent")}</Muted>
      {error && <Failed>{error}</Failed>}
    </Section>
  );
}
