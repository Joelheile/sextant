import { useEffect, useState } from "react";
import { api, type Settings } from "./api";
import { t } from "./i18n";
import MapList from "./MapList";
import Onboarding from "./Onboarding";

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.getState().then(setSettings).catch((err) => setError(String(err)));
  }, []);

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
  return (
    <main className="shell">
      <h1>{t("workflowsTitle")}</h1>
      <MapList headline />
    </main>
  );
}
