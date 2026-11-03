export const MB = 1024 * 1024;

export function hours(value: number): string {
  return value.toFixed(1);
}

export function dateTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString(navigator.language, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}
