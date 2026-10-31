import { useEffect, useState } from "react";
import { api, listenDownload, type Models } from "./api";
import { MB } from "./format";
import { t } from "./i18n";
import { Failed, List, Muted, SwitchRow } from "./layout";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";


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

  if (!models) return error ? <Failed>{error}</Failed> : null;

  return (
    <div className="flex flex-col gap-3">
      {models.installed ? (
        <List>
          <SwitchRow label={t("audioSwitch")} checked={models.audio} onChange={(on) => void toggle(on)} />
        </List>
      ) : (
        <>
          <p className="text-sm">{t("audioBody")}</p>
          <Muted>{t("audioDownload", { size: Math.round(models.bytes / MB) })}</Muted>
          <Muted>{t("audioCredit")}</Muted>
          <Muted>{t("audioLaw")}</Muted>
          {progress ? (
            <div aria-live="polite" className="flex flex-col gap-2">
              <Progress value={(progress.done / progress.total) * 100} />
              <div className="flex items-center justify-between gap-3">
                <span className="text-xs text-muted-foreground tabular-nums">
                  {t("downloading", {
                    done: Math.round(progress.done / MB),
                    total: Math.round(progress.total / MB),
                  })}
                </span>
                <Button type="button" variant="ghost" size="sm" onClick={() => void api.cancelDownload()}>
                  {t("cancel")}
                </Button>
              </div>
            </div>
          ) : (
            <Button type="button" className="self-start" onClick={() => void download()}>
              {t("audioDownloadButton")}
            </Button>
          )}
        </>
      )}
      {error && <Failed>{error}</Failed>}
    </div>
  );
}
