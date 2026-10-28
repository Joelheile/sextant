import { t } from "./i18n";

export function hours(value: number): string {
  return value.toFixed(1);
}

/** `90` becomes "2 min". Rounds to the largest fitting unit. */
export function duration(seconds: number): string {
  if (seconds < 60) return t("durSeconds", { n: Math.round(seconds) });
  if (seconds < 3600) return t("durMinutes", { n: Math.round(seconds / 60) });
  if (seconds < 86400) return t("durHours", { n: Math.round(seconds / 3600) });
  return t("durDays", { n: Math.round(seconds / 86400) });
}

export function shortDate(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString(navigator.language, {
    day: "numeric",
    month: "short",
  });
}

export function dateTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString(navigator.language, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}
