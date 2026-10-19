/** Small helpers for showing history entries. */

/** The first lines of a statement for the list, with a marker when something was cut. */
export function preview(sql: string, maxLines = 4, maxChars = 360): string {
  const lines = sql.trim().split(/\r?\n/);
  let text = lines.slice(0, maxLines).join('\n');
  let cut = lines.length > maxLines;
  if (text.length > maxChars) {
    text = text.slice(0, maxChars);
    cut = true;
  }
  return cut ? `${text.trimEnd()} …` : text;
}

const pad = (n: number) => String(n).padStart(2, '0');
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** "just now", "5 min ago", "14:03" (today), "12 Oct 14:03", "12 Oct 2025". Local time. */
export function whenLabel(at: number, now = Date.now()): string {
  const d = new Date(at);
  const n = new Date(now);
  const diff = now - at;
  if (diff < 45_000) return 'just now';
  if (diff < 3_600_000) return `${Math.max(1, Math.round(diff / 60_000))} min ago`;
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (d.toDateString() === n.toDateString()) return time;
  if (d.getFullYear() === n.getFullYear()) return `${d.getDate()} ${MONTHS[d.getMonth()]} ${time}`;
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
}

export function durationLabel(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2)} s`;
}
