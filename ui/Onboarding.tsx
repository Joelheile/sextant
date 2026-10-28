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
import MapList from "./MapList";
import Permissions from "./Permissions";

const STEPS = 5;

function Frame({ step, title, children }: { step: number; title: string; children: ReactNode }) {
  return (
    <main className="shell">
      <p className="muted">{t("step", { n: step, total: STEPS })}</p>
      <h1>{title}</h1>
      {children}
    </main>
  );
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
      <p>{t("sourcesBody")}</p>
      <ul className="choices">
        {CHOICES.map(({ key, title, note }) => (
          <li key={key}>
            <label>
              <input
                type="checkbox"
                checked={sources[key]}
                onChange={(e) => setSources({ ...sources, [key]: e.target.checked })}
              />
              <span>
                <strong>{t(title)}</strong>
                <span className="muted">{t(note)}</span>
              </span>
            </label>
            {key === "git" && sources.git && (
              <div className="folders">
                {folders.map((folder) => (
                  <div key={folder} className="folder">
                    <code>{folder}</code>
                    <button
                      type="button"
                      className="link"
                      onClick={() => setFolders(folders.filter((f) => f !== folder))}
                    >
                      {t("removeFolder")}
                    </button>
                  </div>
                ))}
                <button type="button" className="secondary" onClick={() => void addFolder()}>
                  {t("addFolder")}
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>
      {error && <p role="alert">{error}</p>}
      <div className="actions">
        <button type="button" onClick={() => void next()}>
          {t("next")}
        </button>
      </div>
    </Frame>
  );
}

function GrantAccess({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  return (
    <Frame step={2} title={t("accessTitle")}>
      <p>{t("accessBody")}</p>
      <Permissions />
      <div className="actions">
        <button type="button" className="secondary" onClick={onBack}>
          {t("back")}
        </button>
        <button type="button" onClick={onNext}>
          {t("readHistory")}
        </button>
      </div>
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
        <>
          <p>{t("mapWorking")}</p>
          {backfill.stage && (
            <p className="muted" aria-live="polite">
              {t(`stage_${backfill.stage}`)}
            </p>
          )}
        </>
      )}
      {backfill.state === "failed" && (
        <p role="alert">{t("mapFailed", { message: backfill.message })}</p>
      )}
      {backfill.state === "done" && (
        <>
          <p className="muted">{t("mapRough")}</p>
          <MapList headline />
          <ul className="skipped muted">
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
      <div className="actions">
        <button type="button" disabled={backfill.state === "running"} onClick={onNext}>
          {t("next")}
        </button>
      </div>
    </Frame>
  );
}

function Audio({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  return (
    <Frame step={4} title={t("audioTitle")}>
      <CallAudio />
      <div className="actions">
        <button type="button" className="secondary" onClick={onBack}>
          {t("back")}
        </button>
        <button type="button" onClick={onNext}>
          {t("next")}
        </button>
      </div>
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
      <p>{t("connectBody")}</p>

      <section>
        <h2>{t("desktopTitle")}</h2>
        <button type="button" onClick={() => void addToDesktop()} disabled={!info}>
          {t("desktopAdd")}
        </button>
        {written && (
          <p className="ok" aria-live="polite">
            {written.backup
              ? t("desktopDone", { backup: written.backup })
              : t("desktopDoneNoBackup", { path: written.path })}
          </p>
        )}
      </section>

      <section>
        <h2>{t("codeTitle")}</h2>
        <p className="muted">{t("codeBody")}</p>
        {info && (
          <div className="command">
            <code>{info.claude_code_command}</code>
            <button type="button" className="secondary" onClick={() => void copy()}>
              {copied ? t("copied") : t("copy")}
            </button>
          </div>
        )}
      </section>

      {settings && (
        <section>
          <label className="switch">
            <input
              type="checkbox"
              checked={settings.mcp_rewind}
              onChange={(e) => void switches(e.target.checked, settings.mcp_workflows)}
            />
            {t("switchRewind")}
          </label>
          <label className="switch">
            <input
              type="checkbox"
              checked={settings.mcp_workflows}
              onChange={(e) => void switches(settings.mcp_rewind, e.target.checked)}
            />
            {t("switchWorkflows")}
          </label>
        </section>
      )}

      <p className="muted">{t("cloudNote")}</p>
      {error && <p role="alert">{error}</p>}
      <div className="actions">
        <button type="button" onClick={() => void finish()}>
          {t("finish")}
        </button>
      </div>
    </Frame>
  );
}

export default function Onboarding({ settings, onDone }: { settings: Settings; onDone: () => void }) {
  const [step, setStep] = useState(1);
  if (step === 1) return <PickSources settings={settings} onNext={() => setStep(2)} />;
  if (step === 2) return <GrantAccess onBack={() => setStep(1)} onNext={() => setStep(3)} />;
  if (step === 3) return <FirstMap onNext={() => setStep(4)} />;
  if (step === 4) return <Audio onBack={() => setStep(3)} onNext={() => setStep(5)} />;
  return <ConnectToClaude onFinish={onDone} />;
}
