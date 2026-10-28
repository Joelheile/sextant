import { useEffect, useState } from "react";
import { api, listenDownload, type Models } from "./api";
import { t } from "./i18n";

const MB = 1024 * 1024;

/** Download consent and the on/off switch for call transcription (plan.md §4). */
export default function CallAudio() {
  const [models, setModels] = useState<Models | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = () =>
    api
      .modelsStatus()
      .then(setModels)
      .catch((err) => setError(String(err)));

  useEffect(() => {
    let cancelled = false;
    let stop: (() => void) | undefined;
    void listenDownload({
      progress: (done, total) => setProgress({ done, total }),
      done: () => {
        setProgress(null);
        api
          .setAudio(true)
          .then(reload)
          .catch((err) => setError(String(err)));
      },
      failed: (message, wasCancelled) => {
        setProgress(null);
        if (!wasCancelled) setError(t("downloadFailed", { message }));
        void reload();
      },
    }).then((unlisten) => {
      if (cancelled) return unlisten();
      stop = unlisten;
      void reload();
    });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  const download = async () => {
    setError(null);
    setProgress({ done: 0, total: models?.bytes ?? 1 });
    try {
      await api.downloadModels();
    } catch (err) {
      setProgress(null);
      setError(String(err));
    }
  };

  const toggle = async (on: boolean) => {
    try {
      await api.setAudio(on);
      await reload();
    } catch (err) {
      setError(String(err));
    }
  };

  if (!models) return error ? <p role="alert">{error}</p> : null;

  return (
    <section className="call-audio">
      {models.installed ? (
        <label className="switch">
          <input
            type="checkbox"
            checked={models.audio}
            onChange={(e) => void toggle(e.target.checked)}
          />
          {t("audioSwitch")}
        </label>
      ) : (
        <>
          <p>{t("audioBody")}</p>
          <p>{t("audioDownload", { size: Math.round(models.bytes / MB) })}</p>
          <p className="muted">{t("audioCredit")}</p>
          <p className="muted">{t("audioLaw")}</p>
          {progress ? (
            <div aria-live="polite">
              <progress value={progress.done} max={progress.total} />
              <span className="muted">
                {t("downloading", {
                  done: Math.round(progress.done / MB),
                  total: Math.round(progress.total / MB),
                })}
              </span>
              <button type="button" className="secondary" onClick={() => void api.cancelDownload()}>
                {t("cancel")}
              </button>
            </div>
          ) : (
            <button type="button" onClick={() => void download()}>
              {t("audioDownloadButton")}
            </button>
          )}
        </>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
