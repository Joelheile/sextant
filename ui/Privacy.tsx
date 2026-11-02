import { useEffect, useState } from "react";
import { api, type Live, type Settings } from "./api";
import CallAudio from "./CallAudio";
import { X } from "lucide-react";
import { MB } from "./format";
import { t, type Key } from "./i18n";
import { Failed, List, Muted, Section, SwitchRow } from "./layout";
import Permissions from "./Permissions";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";


const LIVE_SWITCHES: { key: keyof Omit<Live, "audio">; label: Key }[] = [
  { key: "metadata", label: "liveMetadata" },
  { key: "ocr", label: "liveOcr" },
  { key: "copy", label: "liveCopy" },
  { key: "actions", label: "liveActions" },
  { key: "opaque_browsers", label: "liveOpaque" },
];

const RETENTION: { value: string; days: number | null; label: Key }[] = [
  { value: "30", days: 30, label: "retention_30" },
  { value: "90", days: 90, label: "retention_90" },
  { value: "365", days: 365, label: "retention_365" },
  { value: "forever", days: null, label: "retention_forever" },
];

function ExcludedApps({
  apps,
  onChange,
}: {
  apps: string[];
  onChange: (apps: string[]) => void;
}) {
  const [name, setName] = useState("");
  return (
    <Section title={t("excludedTitle")} description={t("excludedBody")}>
      {apps.length > 0 && (
        <ul className="flex flex-wrap gap-1.5">
          {apps.map((app) => (
            <li key={app}>
              <Badge variant="secondary" className="h-6 gap-0.5 pr-0.5 text-sm font-normal">
                {app}
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  className="size-5 rounded-full"
                  aria-label={`${t("removeFolder")} ${app}`}
                  onClick={() => onChange(apps.filter((a) => a !== app))}
                >
                  <X />
                </Button>
              </Badge>
            </li>
          ))}
        </ul>
      )}
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          const added = name.trim();
          if (added && !apps.includes(added)) onChange([...apps, added]);
          setName("");
        }}
      >
        <Input
          type="text"
          value={name}
          placeholder={t("excludedPlaceholder")}
          aria-label={t("excludedPlaceholder")}
          onChange={(e) => setName(e.target.value)}
        />
        <Button type="submit" variant="outline" disabled={name.trim() === ""}>
          {t("excludedAdd")}
        </Button>
      </form>
    </Section>
  );
}

function DeleteEverything() {
  const [confirming, setConfirming] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const confirm = async () => {
    setWorking(true);
    try {
      // The app restarts when this succeeds.
      await api.deleteEverything();
    } catch (err) {
      setWorking(false);
      setError(String(err));
    }
  };

  return (
    <Section title={t("deleteTitle")} description={t("deleteBody")}>
      {confirming ? (
        <div
          role="alertdialog"
          aria-label={t("deleteTitle")}
          className="flex flex-col gap-3 rounded-xl bg-destructive/5 p-4 ring-1 ring-destructive/30"
        >
          <p className="text-sm">{t("deleteConfirm")}</p>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              className="bg-destructive text-white hover:bg-destructive/90"
              disabled={working}
              onClick={() => void confirm()}
            >
              {working ? t("deleteWorking") : t("deleteYes")}
            </Button>
            <Button type="button" variant="outline" disabled={working} onClick={() => setConfirming(false)}>
              {t("cancel")}
            </Button>
          </div>
        </div>
      ) : (
        <Button type="button" variant="destructive" className="self-start" onClick={() => setConfirming(true)}>
          {t("deleteButton")}
        </Button>
      )}
      {error && <Failed>{error}</Failed>}
    </Section>
  );
}

function DebugReport() {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const copy = async () => {
    try {
      // The clipboard write starts inside the click, which WebKit needs; the text follows.
      const report = api.debugReport().then((text) => new Blob([text], { type: "text/plain" }));
      await navigator.clipboard.write([new ClipboardItem({ "text/plain": report })]);
      setCopied(true);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <Section title={t("debugTitle")} description={t("debugBody")}>
      <Button type="button" variant="outline" className="self-start" onClick={() => void copy()}>
        {copied ? t("copied") : t("debugCopy")}
      </Button>
      {error && <Failed>{error}</Failed>}
    </Section>
  );
}

/** Sources, permissions, exclusions, retention, agent access, the debug report and Delete everything. */
export default function Privacy() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [bytes, setBytes] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.getState(), api.storageUsed()])
      .then(([state, storage]) => {
        setSettings(state);
        setBytes(storage.bytes);
      })
      .catch((err) => setError(String(err)));
  }, []);

  const change = (saving: Promise<Settings>) =>
    saving
      .then((saved) => {
        setSettings(saved);
        setError(null);
      })
      .catch((err) => setError(String(err)));

  if (!settings) return error ? <Failed>{error}</Failed> : <Muted>{t("loading")}</Muted>;

  return (
    <>
      <Section title={t("privacySources")}>
        <Permissions />
        <List>
          {LIVE_SWITCHES.map(({ key, label }) => (
            <SwitchRow
              key={key}
              label={t(label)}
              checked={settings.live[key]}
              onChange={(on) => void change(api.setLive({ ...settings.live, [key]: on }))}
            />
          ))}
        </List>
      </Section>

      <ExcludedApps
        apps={settings.excluded_apps}
        onChange={(apps) => void change(api.setExcludedApps(apps))}
      />

      <List>
        <li className="flex items-center justify-between gap-4 px-4 py-3">
          <span className="text-sm">{t("retentionTitle")}</span>
          <Select
            value={settings.retention_days === null ? "forever" : String(settings.retention_days)}
            onValueChange={(value) =>
              void change(api.setRetention(RETENTION.find((r) => r.value === value)?.days ?? null))
            }
          >
            <SelectTrigger aria-label={t("retentionTitle")} className="min-w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {RETENTION.map(({ value, label }) => (
                <SelectItem key={value} value={value}>
                  {t(label)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </li>
        <li className="flex items-center justify-between gap-4 px-4 py-3">
          <span className="text-sm">{t("storageTitle")}</span>
          <span className="text-sm text-muted-foreground tabular-nums">
            {bytes === null ? t("loading") : t("storageValue", { size: (bytes / MB).toFixed(1) })}
          </span>
        </li>
      </List>

      <Section title={t("mcpTitle")} description={t("cloudNote")}>
        <List>
          <SwitchRow
            label={t("switchRewind")}
            checked={settings.mcp_rewind}
            onChange={(on) => void change(api.setMcpSwitches(on, settings.mcp_workflows))}
          />
          <SwitchRow
            label={t("switchWorkflows")}
            checked={settings.mcp_workflows}
            onChange={(on) => void change(api.setMcpSwitches(settings.mcp_rewind, on))}
          />
        </List>
      </Section>

      <Section title={t("audioTitle")}>
        <CallAudio />
      </Section>

      {error && <Failed>{error}</Failed>}
      <DebugReport />
      <DeleteEverything />

      <Section title={t("aboutTitle")} description={t("aboutBody")} />
    </>
  );
}
