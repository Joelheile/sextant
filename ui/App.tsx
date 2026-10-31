import { useEffect, useState } from "react";
import { api, onReady, type Settings } from "./api";
import { t, type Key } from "./i18n";
import { Failed, Muted, Page } from "./layout";
import Onboarding from "./Onboarding";
import Results from "./Results";
import Workflows from "./Workflows";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

const TABS = [
  { id: "workflows", label: "tabWorkflows", screen: Workflows },
  { id: "results", label: "tabResults", screen: Results },
] as const satisfies readonly { id: string; label: Key; screen: () => React.ReactNode }[];

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
    return <Onboarding settings={settings} onDone={() => setSettings({ ...settings, onboarded: true })} />;
  }
  return (
    <Page>
      <Tabs defaultValue="workflows" className="gap-8">
        <TabsList variant="line" aria-label="Sextant" className="-ml-1.5">
          {TABS.map((tab) => (
            <TabsTrigger key={tab.id} value={tab.id} className="px-2.5">
              {t(tab.label)}
            </TabsTrigger>
          ))}
        </TabsList>
        {TABS.map(({ id, screen: Screen }) => (
          <TabsContent key={id} value={id} className="flex flex-col gap-8">
            <Screen />
          </TabsContent>
        ))}
      </Tabs>
    </Page>
  );
}
