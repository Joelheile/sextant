import { useEffect, useRef, useState } from "react";
import { api, type WeeklyReport } from "./api";
import { hours, shortDate } from "./format";
import { t } from "./i18n";
import { Done, Failed, Section } from "./layout";
import { Button } from "@/components/ui/button";

const WIDTH = 1200;
const HEIGHT = 630;
const INK = "#1c1917";
const MUTED = "#78716c";
const ACCENT = "#0f766e";
const FONT = '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';

function fitText(ctx: CanvasRenderingContext2D, text: string, size: number, maxWidth: number) {
  let current = size;
  ctx.font = `600 ${current}px ${FONT}`;
  while (ctx.measureText(text).width > maxWidth && current > 20) {
    current -= 2;
    ctx.font = `600 ${current}px ${FONT}`;
  }
}

/** Draws hours and kinds of work only. The report data holds nothing else. */
function draw(canvas: HTMLCanvasElement, report: WeeklyReport) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.fillStyle = "#fafaf9";
  ctx.fillRect(0, 0, WIDTH, HEIGHT);
  ctx.textBaseline = "alphabetic";

  ctx.fillStyle = ACCENT;
  ctx.font = `700 34px ${FONT}`;
  ctx.fillText("Sextant", 60, 90);
  ctx.fillStyle = MUTED;
  ctx.font = `400 28px ${FONT}`;
  ctx.textAlign = "right";
  ctx.fillText(t("reportCaption", { date: shortDate(report.from_ts) }), WIDTH - 60, 90);
  ctx.textAlign = "left";

  ctx.fillStyle = INK;
  fitText(ctx, t("reportHeadline", { hours: hours(report.hours_back) }), 64, WIDTH - 120);
  ctx.fillText(t("reportHeadline", { hours: hours(report.hours_back) }), 60, 190);
  if (report.hours_saved > 0) {
    ctx.fillStyle = ACCENT;
    ctx.font = `500 34px ${FONT}`;
    ctx.fillText(t("reportSavedLine", { hours: hours(report.hours_saved) }), 60, 250);
  }

  if (report.top.length === 0) {
    ctx.fillStyle = MUTED;
    ctx.font = `400 32px ${FONT}`;
    ctx.fillText(t("reportEmpty"), 60, 360);
    return;
  }
  report.top.forEach((row, index) => {
    const y = 340 + index * 90;
    ctx.fillStyle = INK;
    const chain = row.categories.map((c) => t(`cat_${c}`)).join("  ›  ");
    fitText(ctx, chain, 36, 820);
    ctx.fillText(chain, 60, y);
    ctx.fillStyle = ACCENT;
    ctx.font = `700 36px ${FONT}`;
    ctx.textAlign = "right";
    ctx.fillText(t("hoursWeek", { hours: hours(row.hours_week) }), WIDTH - 60, y);
    ctx.textAlign = "left";
    ctx.fillStyle = "#e7e5e4";
    ctx.fillRect(60, y + 28, WIDTH - 120, 2);
  });
}

function localDay(): string {
  const now = new Date();
  const two = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${two(now.getMonth() + 1)}-${two(now.getDate())}`;
}

/** The redacted weekly report as an image. Copy and Save act only when the user clicks. */
export default function Report() {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [report, setReport] = useState<WeeklyReport | null>(null);
  const [image, setImage] = useState<Blob | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .weeklyReport()
      .then(setReport)
      .catch((err) => setError(String(err)));
  }, []);

  useEffect(() => {
    if (!report || !canvas.current) return;
    draw(canvas.current, report);
    canvas.current.toBlob(setImage, "image/png");
  }, [report]);

  const copy = async () => {
    if (!image) return;
    try {
      await navigator.clipboard.write([new ClipboardItem({ "image/png": image })]);
      setMessage(t("reportCopied"));
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  };

  const save = async () => {
    if (!image) return;
    try {
      const bytes = Array.from(new Uint8Array(await image.arrayBuffer()));
      setMessage(t("reportSaved", { path: await api.saveReportImage(localDay(), bytes) }));
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <Section title={t("reportTitle")} description={t("reportHint")}>
      <canvas
        ref={canvas}
        className="block h-auto w-full rounded-xl ring-1 ring-foreground/10"
        width={WIDTH}
        height={HEIGHT}
        role="img"
        aria-label={t("reportAlt")}
      />
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" disabled={!image} onClick={() => void copy()}>
          {t("reportCopy")}
        </Button>
        <Button type="button" variant="outline" disabled={!image} onClick={() => void save()}>
          {t("reportSave")}
        </Button>
      </div>
      {message && <Done>{message}</Done>}
      {error && <Failed>{error}</Failed>}
    </Section>
  );
}
