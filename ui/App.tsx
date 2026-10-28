import { useEffect, useState } from "react";
import { api, onReady, type Settings } from "./api";
import { t, type Key } from "./i18n";
import Onboarding from "./Onboarding";
import Privacy from "./Privacy";
import Results from "./Results";
import Search from "./Search";
import Workflows from "./Workflows";

const TABS = [
  { id: "workflows", label: "tabWorkflows", screen: Workflows },
  { id: "search", label: "tabSearch", screen: Search },
  { id: "results", label: "tabResults", screen: Results },
  { id: "privacy", label: "tabPrivacy", screen: Privacy },
] as const satisfies readonly { id: string; label: Key; screen: () => React.ReactNode }[];

export default function App() {
  const [ready, setReady] = useState(false);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<(typeof TABS)[number]["id"]>("workflows");

  useEffect(() => {
    const check = () =>
      api.engineStatus().then(setReady).catch((err) => setError(String(err)));
    const stop = onReady(check).then((unlisten) => {
      check();
      return unlisten;
    });
    return () => {
      stop.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (ready) {
      api.getState().then(setSettings).catch((err) => setError(String(err)));
    }
  }, [ready]);

  if (error) {
    return (
      <main className="shell">
        <h1>{t("errorTitle")}</h1>
        <p role="alert">{error}</p>
      </main>
    );
  }
  if (!settings) {
    return (
      <main className="shell">
        <p className="muted">{t("loading")}</p>
      </main>
    );
  }
  if (!settings.onboarded) {
    return <Onboarding settings={settings} onDone={() => setSettings({ ...settings, onboarded: true })} />;
  }
  const Screen = TABS.find((candidate) => candidate.id === tab)?.screen ?? Workflows;
  return (
    <main className="shell">
      <nav className="tabs" aria-label="Sextant">
        {TABS.map((candidate) => (
          <button
            key={candidate.id}
            type="button"
            className={candidate.id === tab ? "tab current" : "tab"}
            aria-current={candidate.id === tab ? "page" : undefined}
            onClick={() => setTab(candidate.id)}
          >
            {t(candidate.label)}
          </button>
        ))}
      </nav>
      <Screen />
    </main>
  );
}
