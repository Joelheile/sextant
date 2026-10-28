import { useEffect, useState } from "react";
import { api, type Live, type Settings } from "./api";
import CallAudio from "./CallAudio";
import { t, type Key } from "./i18n";
import Permissions from "./Permissions";

const MB = 1024 * 1024;

const LIVE_SWITCHES: { key: keyof Omit<Live, "audio">; label: Key }[] = [
  { key: "metadata", label: "liveMetadata" },
  { key: "ocr", label: "liveOcr" },
  { key: "copy", label: "liveCopy" },
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
    <section>
      <h2>{t("excludedTitle")}</h2>
      <p className="muted">{t("excludedBody")}</p>
      <ul className="chips">
        {apps.map((app) => (
          <li key={app}>
            {app}
            <button
              type="button"
              className="link"
              aria-label={`${t("removeFolder")} ${app}`}
              onClick={() => onChange(apps.filter((a) => a !== app))}
            >
              {t("removeFolder")}
            </button>
          </li>
        ))}
      </ul>
      <form
        className="inline"
        onSubmit={(e) => {
          e.preventDefault();
          const added = name.trim();
          if (added && !apps.includes(added)) onChange([...apps, added]);
          setName("");
        }}
      >
        <input
          type="text"
          value={name}
          placeholder={t("excludedPlaceholder")}
          aria-label={t("excludedPlaceholder")}
          onChange={(e) => setName(e.target.value)}
        />
        <button type="submit" className="secondary" disabled={name.trim() === ""}>
          {t("excludedAdd")}
        </button>
      </form>
    </section>
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
    <section>
      <h2>{t("deleteTitle")}</h2>
      <p className="muted">{t("deleteBody")}</p>
      {confirming ? (
        <div role="alertdialog" aria-label={t("deleteTitle")} className="confirm">
          <p>{t("deleteConfirm")}</p>
          <div className="actions-row">
            <button type="button" className="danger" disabled={working} onClick={() => void confirm()}>
              {working ? t("deleteWorking") : t("deleteYes")}
            </button>
            <button
              type="button"
              className="secondary"
              disabled={working}
              onClick={() => setConfirming(false)}
            >
              {t("cancel")}
            </button>
          </div>
        </div>
      ) : (
        <button type="button" className="danger" onClick={() => setConfirming(true)}>
          {t("deleteButton")}
        </button>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

/** Sources, permissions, exclusions, retention, agent access and Delete everything. */
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

  if (!settings) return error ? <p role="alert">{error}</p> : <p className="muted">{t("loading")}</p>;

  return (
    <>
      <h2>{t("privacySources")}</h2>
      <Permissions />
      <section>
        {LIVE_SWITCHES.map(({ key, label }) => (
          <label key={key} className="switch">
            <input
              type="checkbox"
              checked={settings.live[key]}
              onChange={(e) =>
                void change(api.setLive({ ...settings.live, [key]: e.target.checked }))
              }
            />
            {t(label)}
          </label>
        ))}
      </section>

      <ExcludedApps
        apps={settings.excluded_apps}
        onChange={(apps) => void change(api.setExcludedApps(apps))}
      />

      <section>
        <h2>{t("retentionTitle")}</h2>
        <select
          aria-label={t("retentionTitle")}
          value={settings.retention_days === null ? "forever" : String(settings.retention_days)}
          onChange={(e) =>
            void change(
              api.setRetention(RETENTION.find((r) => r.value === e.target.value)?.days ?? null),
            )
          }
        >
          {RETENTION.map(({ value, label }) => (
            <option key={value} value={value}>
              {t(label)}
            </option>
          ))}
        </select>
      </section>

      <section>
        <h2>{t("storageTitle")}</h2>
        <p>{bytes === null ? t("loading") : t("storageValue", { size: (bytes / MB).toFixed(1) })}</p>
      </section>

      <section>
        <h2>{t("mcpTitle")}</h2>
        <label className="switch">
          <input
            type="checkbox"
            checked={settings.mcp_rewind}
            onChange={(e) =>
              void change(api.setMcpSwitches(e.target.checked, settings.mcp_workflows))
            }
          />
          {t("switchRewind")}
        </label>
        <label className="switch">
          <input
            type="checkbox"
            checked={settings.mcp_workflows}
            onChange={(e) => void change(api.setMcpSwitches(settings.mcp_rewind, e.target.checked))}
          />
          {t("switchWorkflows")}
        </label>
        <p className="muted">{t("cloudNote")}</p>
      </section>

      <section>
        <h2>{t("audioTitle")}</h2>
        <CallAudio />
      </section>

      {error && <p role="alert">{error}</p>}
      <DeleteEverything />

      <section>
        <h2>{t("aboutTitle")}</h2>
        <p className="muted">{t("aboutBody")}</p>
      </section>
    </>
  );
}
