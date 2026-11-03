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
import { Done, Failed, List } from "./layout";
import { AllowButton, PERMISSION_LABEL, PermissionNotes, usePermissions } from "./Permissions";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";

const STEPS = ["sources", "access", "map", "connect"] as const;
type Step = (typeof STEPS)[number];

/** The step's buttons, pinned bottom right so they stay put from step to step. */
function Actions({ children }: { children: ReactNode }) {
  return <div className="mt-auto flex justify-end gap-2 pt-4">{children}</div>;
}

const CHOICES: { key: keyof Sources; title: Key }[] = [
  { key: "browsers", title: "sourceBrowsers" },
  { key: "safari", title: "sourceSafari" },
  { key: "calendar", title: "sourceCalendar" },
  { key: "git", title: "sourceGit" },
  { key: "files", title: "sourceFiles" },
];

type Choices = { sources: Sources; folders: string[] };

function PickSources({
  choices,
  onChange,
  onNext,
}: {
  choices: Choices;
  onChange: (choices: Choices) => void;
  onNext: () => void;
}) {
  const { sources, folders } = choices;
  const [error, setError] = useState<string | null>(null);

  const addFolder = async () => {
    try {
      const folder = await api.pickFolder(t("pickCodeFolder"));
      if (folder && !folders.includes(folder)) onChange({ sources, folders: [...folders, folder] });
    } catch (err) {
      setError(String(err));
    }
  };

  /** Saves before moving on, because the access step asks only for what these sources need. */
  const next = async () => {
    try {
      await api.saveChoices(sources, folders);
      onNext();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <>
      <List>
        {CHOICES.map(({ key, title }) => (
          <li key={key} className="flex flex-col gap-2 px-4 py-2">
            <div className="flex min-h-8 items-center justify-between gap-4">
              <Label className="font-normal">
                <Checkbox
                  checked={sources[key]}
                  onCheckedChange={(on) =>
                    onChange({
                      sources: { ...sources, [key]: on === true },
                      folders,
                    })
                  }
                />
                <span className="font-medium">{t(title)}</span>
              </Label>
              {key === "git" && sources.git && (
                <Button type="button" variant="outline" size="sm" onClick={() => void addFolder()}>
                  {t("addFolder")}
                </Button>
              )}
            </div>
            {key === "git" && sources.git && folders.length > 0 && (
              <ul className="ml-6 flex flex-col gap-1.5">
                {folders.map((folder) => (
                  <li key={folder} className="flex items-center gap-2">
                    <code className="font-mono text-xs break-all">{folder}</code>
                    <Button
                      type="button"
                      variant="ghost"
                      size="xs"
                      onClick={() =>
                        onChange({
                          sources,
                          folders: folders.filter((f) => f !== folder),
                        })
                      }
                    >
                      {t("removeFolder")}
                    </Button>
                  </li>
                ))}
              </ul>
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
    </>
  );
}

function GrantAccess({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  const state = usePermissions();
  return (
    <>
      {state.permissions && state.permissions.length > 0 && (
        <List>
          {state.permissions.map((permission) => (
            <li key={permission.name} className="flex min-h-12 items-center justify-between gap-4 px-4 py-2">
              <span className="text-sm font-medium">{t(PERMISSION_LABEL[permission.name])}</span>
              <AllowButton state={state} names={[permission.name]} />
            </li>
          ))}
        </List>
      )}
      <PermissionNotes state={state} />
      <Actions>
        <Button type="button" variant="ghost" onClick={onBack}>
          {t("back")}
        </Button>
        <Button type="button" onClick={onNext}>
          {t("readHistory")}
        </Button>
      </Actions>
    </>
  );
}

type Backfill =
  | { state: "running"; stage: Stage | null }
  | { state: "done"; summary: BackfillDone }
  | { state: "failed"; message: string };

function FirstMap({ onNext }: { onNext: () => void }) {
  const [backfill, setBackfill] = useState<Backfill>({
    state: "running",
    stage: null,
  });

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
    <>
      {backfill.state === "running" && (
        <p className="text-sm text-muted-foreground" aria-live="polite">
          {backfill.stage ? t(`stage_${backfill.stage}`) : t("mapWorking")}
        </p>
      )}
      {backfill.state === "failed" && <Failed>{t("mapFailed", { message: backfill.message })}</Failed>}
      {backfill.state === "done" && (
        <>
          <p className="text-sm">
            {t("mapDone", { events: backfill.summary.events, tasks: backfill.summary.tasks })}
          </p>
          <ul className="flex flex-col gap-1 text-xs text-muted-foreground">
            {backfill.summary.sources.flatMap((source) =>
              source.errors.map((message) => (
                <li key={`${source.stage}-${message}`}>
                  {t("sourceSkipped", {
                    stage: t(`stage_${source.stage}`),
                    message,
                  })}
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
    </>
  );
}

/** The agent switches and call audio live in Privacy, so this step only connects. */
function ConnectToClaude({ onFinish }: { onFinish: () => void }) {
  const [info, setInfo] = useState<ConnectInfo | null>(null);
  const [written, setWritten] = useState<Written | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .connectInfo()
      .then(setInfo)
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

  const finish = async () => {
    try {
      await api.finishOnboarding();
      onFinish();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <>
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
      {info && (
        <div className="flex items-center justify-between gap-3 rounded-xl bg-muted px-4 py-2.5">
          <code className="font-mono text-xs break-all">{info.claude_code_command}</code>
          <Button type="button" variant="outline" size="sm" onClick={() => void copy()}>
            {copied ? t("copied") : t("copy")}
          </Button>
        </div>
      )}
      {error && <Failed>{error}</Failed>}
      <Actions>
        <Button type="button" onClick={() => void finish()}>
          {t("finish")}
        </Button>
      </Actions>
    </>
  );
}

/** Keeps the step across "Reopen Sextant", which restarts the app during the access step. It
 * stores the step's name, so a number saved by an older build starts over at sources instead
 * of opening the map, which starts the backfill. */
const STEP_KEY = "onboardingStep";

function savedStep(): Step {
  return STEPS.find((name) => name === localStorage.getItem(STEP_KEY)) ?? "sources";
}

export default function Onboarding({ settings, onDone }: { settings: Settings; onDone: () => void }) {
  const [step, setStepState] = useState(savedStep);
  const [choices, setChoices] = useState<Choices>({
    sources: settings.sources,
    folders: settings.git_folders,
  });
  const setStep = (next: Step) => {
    localStorage.setItem(STEP_KEY, next);
    setStepState(next);
  };
  const finish = () => {
    localStorage.removeItem(STEP_KEY);
    onDone();
  };

  const screens: Record<Step, ReactNode> = {
    sources: <PickSources choices={choices} onChange={setChoices} onNext={() => setStep("access")} />,
    access: <GrantAccess onBack={() => setStep("sources")} onNext={() => setStep("map")} />,
    map: <FirstMap onNext={() => setStep("connect")} />,
    connect: <ConnectToClaude onFinish={finish} />,
  };

  return (
    <main className="mx-auto flex min-h-screen w-full max-w-xl flex-col gap-8 px-6 pt-6 pb-8">
      <Progress
        value={((STEPS.indexOf(step) + 1) / STEPS.length) * 100}
        className="max-w-24 *:duration-300 *:ease-(--ease-out)"
      />
      <div
        key={step}
        className="flex flex-1 flex-col gap-4 duration-200 ease-(--ease-out) animate-in fade-in slide-in-from-bottom-1 motion-reduce:slide-in-from-bottom-0"
      >
        {screens[step]}
      </div>
    </main>
  );
}
