import { useEffect, useState } from "react";
import { api, onReady, type Settings } from "./api";
import { t } from "./i18n";
import { Failed, Muted, Page } from "./layout";
import Onboarding from "./Onboarding";
import Privacy from "./Privacy";
import Routines, { AgentBridge } from "./Routines";

export default function App() {
  const [ready, setReady] = useState(false);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);

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
      <Page>
        <h1 className="text-2xl font-semibold tracking-tight">{t("errorTitle")}</h1>
        <Failed>{error}</Failed>
      </Page>
    );
  }
  if (!settings) {
    return (
      <Page>
        <Muted>{t("loading")}</Muted>
      </Page>
    );
  }
  if (!settings.onboarded) {
    return <Onboarding onDone={() => setSettings({ ...settings, onboarded: true })} />;
  }
  return (
    <Page>
      <Routines />
      <AgentBridge />
      <Privacy />
    </Page>
  );
}
