import { useEffect, useState, type ReactNode } from "react";
import { api, type ConnectInfo, type Written } from "./api";
import { t } from "./i18n";
import { Done, Failed, List } from "./layout";
import { AllowButton, PERMISSION_LABEL, PermissionNotes, usePermissions } from "./Permissions";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";

const STEPS = ["access", "connect"] as const;
type Step = (typeof STEPS)[number];

/** The step's buttons, pinned bottom right so they stay put from step to step. */
function Actions({ children }: { children: ReactNode }) {
  return <div className="mt-auto flex justify-end gap-2 pt-4">{children}</div>;
}

function GrantAccess({ onNext }: { onNext: () => void }) {
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
        <Button type="button" onClick={onNext}>
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

/** Keeps the step across "Reopen Sextant", which restarts the app during the access step. */
const STEP_KEY = "onboardingStep";

function savedStep(): Step {
  return STEPS.find((name) => name === localStorage.getItem(STEP_KEY)) ?? "access";
}

export default function Onboarding({ onDone }: { onDone: () => void }) {
  const [step, setStepState] = useState(savedStep);
  const setStep = (next: Step) => {
    localStorage.setItem(STEP_KEY, next);
    setStepState(next);
  };
  const finish = () => {
    localStorage.removeItem(STEP_KEY);
    onDone();
  };

  const screens: Record<Step, ReactNode> = {
    access: <GrantAccess onNext={() => setStep("connect")} />,
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
