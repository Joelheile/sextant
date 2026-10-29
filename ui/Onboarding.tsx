import { useEffect, useState, type ReactNode } from "react";
import {
  api,
  listenBackfill,
  type BackfillDone,
  type ConnectInfo,
  type Settings,
  type Sources,
  type Stage,
  type Written,
} from "./api";
import { t, type Key } from "./i18n";
import CallAudio from "./CallAudio";
import { Done, Failed, List, Muted, Page, Section, SwitchRow } from "./layout";
import MapList from "./MapList";
import Permissions from "./Permissions";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";

const STEPS = 5;

function Frame({ step, title, children }: { step: number; title: string; children: ReactNode }) {
  return (
    <Page>
      <header className="flex flex-col gap-3">
        <div className="flex items-center gap-3">
          <Progress value={(step / STEPS) * 100} className="h-1 max-w-24" />
          <span className="text-xs text-muted-foreground tabular-nums">{t("step", { n: step, total: STEPS })}</span>
        </div>
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
      </header>
      {children}
    </Page>
  );
}

/** Back and Next, bottom right. */
function Actions({ children }: { children: ReactNode }) {
  return <div className="flex justify-end gap-2">{children}</div>;
}

type SourceChoice = {
  key: keyof Sources;
  title: Key;
  note: Key;
};

const CHOICES: SourceChoice[] = [
  { key: "browsers", title: "sourceBrowsers", note: "sourceBrowsersNote" },
  { key: "safari", title: "sourceSafari", note: "sourceSafariNote" },
  { key: "calendar", title: "sourceCalendar", note: "sourceCalendarNote" },
  { key: "git", title: "sourceGit", note: "sourceGitNote" },
  { key: "files", title: "sourceFiles", note: "sourceFilesNote" },
];

function PickSources({ settings, onNext }: { settings: Settings; onNext: () => void }) {
  const [sources, setSources] = useState(settings.sources);
  const [folders, setFolders] = useState(settings.git_folders);
  const [error, setError] = useState<string | null>(null);

  const addFolder = async () => {
    try {
      const folder = await api.pickFolder(t("pickCodeFolder"));
      if (folder && !folders.includes(folder)) setFolders([...folders, folder]);
    } catch (err) {
      setError(String(err));
    }
  };

  const next = async () => {
    try {
      await api.saveChoices(sources, folders);
      onNext();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <Frame step={1} title={t("sourcesTitle")}>
      <p className="text-sm">{t("sourcesBody")}</p>
      <List>
        {CHOICES.map(({ key, title, note }) => (
          <li key={key} className="flex flex-col gap-3 px-4 py-3">
            <Label className="items-start font-normal">
              <Checkbox
                className="mt-0.5"
                checked={sources[key]}
                onCheckedChange={(on) => setSources({ ...sources, [key]: on === true })}
              />
              <span className="flex flex-col gap-0.5">
                <span className="font-medium">{t(title)}</span>
                <span className="text-muted-foreground">{t(note)}</span>
              </span>
            </Label>
            {key === "git" && sources.git && (
              <div className="ml-6 flex flex-col items-start gap-1.5">
                {folders.map((folder) => (
                  <div key={folder} className="flex items-center gap-2">
                    <code className="font-mono text-xs break-all">{folder}</code>
                    <Button
                      type="button"
                      variant="ghost"
                      size="xs"
                      onClick={() => setFolders(folders.filter((f) => f !== folder))}
                    >
                      {t("removeFolder")}
                    </Button>
                  </div>
                ))}
                <Button type="button" variant="outline" size="sm" onClick={() => void addFolder()}>
                  {t("addFolder")}
                </Button>
              </div>
            )}
          </li>
        ))}
      </List>
      {error && <Failed>{error}</Failed>}
      <Actions>
        <Button type="button" onClick={() => void next()}>
          {t("next")}
        </Button>
      </Actions>
    </Frame>
  );
}

function GrantAccess({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  return (
    <Frame step={2} title={t("accessTitle")}>
      <p className="text-sm">{t("accessBody")}</p>
      <Permissions />
      <Actions>
        <Button type="button" variant="ghost" onClick={onBack}>
          {t("back")}
        </Button>
        <Button type="button" onClick={onNext}>
          {t("readHistory")}
        </Button>
      </Actions>
    </Frame>
  );
}

type Backfill =
  | { state: "running"; stage: Stage | null }
  | { state: "done"; summary: BackfillDone }
  | { state: "failed"; message: string };

function FirstMap({ onNext }: { onNext: () => void }) {
  const [backfill, setBackfill] = useState<Backfill>({ state: "running", stage: null });

  useEffect(() => {
    let cancelled = false;
    let stop: (() => void) | undefined;
    void listenBackfill({
      progress: (stage) => setBackfill({ state: "running", stage }),
      done: (summary) => setBackfill({ state: "done", summary }),
      failed: (message) => setBackfill({ state: "failed", message }),
    }).then(async (unlisten) => {
      if (cancelled) return unlisten();
      stop = unlisten;
      try {
        await api.startBackfill();
      } catch (err) {
        setBackfill({ state: "failed", message: String(err) });
      }
    });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  return (
    <Frame step={3} title={t("mapTitle")}>
      {backfill.state === "running" && (
        <div className="flex flex-col gap-1">
          <p className="text-sm">{t("mapWorking")}</p>
          {backfill.stage && (
            <p className="text-sm text-muted-foreground" aria-live="polite">
              {t(`stage_${backfill.stage}`)}
            </p>
          )}
        </div>
      )}
      {backfill.state === "failed" && <Failed>{t("mapFailed", { message: backfill.message })}</Failed>}
      {backfill.state === "done" && (
        <>
          <Muted>{t("mapRough")}</Muted>
          <MapList headline />
          <ul className="flex flex-col gap-1 text-xs text-muted-foreground">
            {backfill.summary.sources.flatMap((source) =>
              source.errors.map((message) => (
                <li key={`${source.stage}-${message}`}>
                  {t("sourceSkipped", { stage: t(`stage_${source.stage}`), message })}
                </li>
              )),
            )}
          </ul>
        </>
      )}
      <Actions>
        <Button type="button" disabled={backfill.state === "running"} onClick={onNext}>
          {t("next")}
        </Button>
      </Actions>
    </Frame>
  );
}

function Audio({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  return (
    <Frame step={4} title={t("audioTitle")}>
      <CallAudio />
      <Actions>
        <Button type="button" variant="ghost" onClick={onBack}>
          {t("back")}
        </Button>
        <Button type="button" onClick={onNext}>
          {t("next")}
        </Button>
      </Actions>
    </Frame>
  );
}

function ConnectToClaude({ onFinish }: { onFinish: () => void }) {
  const [info, setInfo] = useState<ConnectInfo | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [written, setWritten] = useState<Written | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.connectInfo(), api.getState()])
      .then(([connect, state]) => {
        setInfo(connect);
        setSettings(state);
      })
      .catch((err) => setError(String(err)));
  }, []);

  const addToDesktop = async () => {
    try {
      setWritten(await api.connectClaudeDesktop());
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  };

  const copy = async () => {
    if (!info) return;
    try {
      await navigator.clipboard.writeText(info.claude_code_command);
      setCopied(true);
    } catch (err) {
      setError(String(err));
    }
  };

  const switches = async (rewind: boolean, workflows: boolean) => {
    try {
      setSettings(await api.setMcpSwitches(rewind, workflows));
    } catch (err) {
      setError(String(err));
    }
  };

  const finish = async () => {
    try {
      await api.finishOnboarding();
      onFinish();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <Frame step={5} title={t("connectTitle")}>
      <p className="text-sm">{t("connectBody")}</p>

      <Section title={t("desktopTitle")}>
        <Button type="button" className="self-start" onClick={() => void addToDesktop()} disabled={!info}>
          {t("desktopAdd")}
        </Button>
        {written && (
          <Done>
            {written.backup
              ? t("desktopDone", { backup: written.backup })
              : t("desktopDoneNoBackup", { path: written.path })}
          </Done>
        )}
      </Section>

      <Section title={t("codeTitle")} description={t("codeBody")}>
        {info && (
          <div className="flex items-center justify-between gap-3 rounded-xl bg-muted px-4 py-2.5">
            <code className="font-mono text-xs break-all">{info.claude_code_command}</code>
            <Button type="button" variant="outline" size="sm" onClick={() => void copy()}>
              {copied ? t("copied") : t("copy")}
            </Button>
          </div>
        )}
      </Section>

      {settings && (
        <Section description={t("cloudNote")}>
          <List>
            <SwitchRow
              label={t("switchRewind")}
              checked={settings.mcp_rewind}
              onChange={(on) => void switches(on, settings.mcp_workflows)}
            />
            <SwitchRow
              label={t("switchWorkflows")}
              checked={settings.mcp_workflows}
              onChange={(on) => void switches(settings.mcp_rewind, on)}
            />
          </List>
        </Section>
      )}

      {error && <Failed>{error}</Failed>}
      <Actions>
        <Button type="button" onClick={() => void finish()}>
          {t("finish")}
        </Button>
      </Actions>
    </Frame>
  );
}

/** Keeps the step across "Reopen Sextant", which restarts the app during step 2. */
const STEP_KEY = "onboardingStep";

export default function Onboarding({ settings, onDone }: { settings: Settings; onDone: () => void }) {
  const [step, setStepState] = useState(() => Number(localStorage.getItem(STEP_KEY)) || 1);
  const setStep = (next: number) => {
    localStorage.setItem(STEP_KEY, String(next));
    setStepState(next);
  };
  const finish = () => {
    localStorage.removeItem(STEP_KEY);
    onDone();
  };
  if (step === 1) return <PickSources settings={settings} onNext={() => setStep(2)} />;
  if (step === 2) return <GrantAccess onBack={() => setStep(1)} onNext={() => setStep(3)} />;
  if (step === 3) return <FirstMap onNext={() => setStep(4)} />;
  if (step === 4) return <Audio onBack={() => setStep(3)} onNext={() => setStep(5)} />;
  return <ConnectToClaude onFinish={finish} />;
}
