import type { ServerMessage } from './types';

export type Tone = 'warn' | 'info' | 'dim';

/** WARNING stands out, NOTICE/INFO are the normal chatter, LOG and DEBUG are background noise. */
export function toneOf(severity: string): Tone {
  const s = severity.toUpperCase();
  if (s === 'WARNING') return 'warn';
  if (s === 'LOG' || s.startsWith('DEBUG')) return 'dim';
  return 'info';
}

/** One message the way psql prints it. */
export function formatMessage(m: ServerMessage): string {
  const lines = [`${m.severity}:  ${m.message}`];
  if (m.detail) lines.push(`DETAIL:  ${m.detail}`);
  if (m.hint) lines.push(`HINT:  ${m.hint}`);
  if (m.context) lines.push(`CONTEXT:  ${m.context.replace(/\n/g, '\n  ')}`);
  return lines.join('\n');
}

export function formatAll(list: readonly ServerMessage[]): string {
  return list.map(formatMessage).join('\n');
}

export function hasWarning(list: readonly ServerMessage[]): boolean {
  return list.some((m) => toneOf(m.severity) === 'warn');
}

/** "+1.2 s" / "+35 ms": when the message arrived relative to the start of the run. */
export function fmtOffset(ms: number): string {
  return ms >= 1000 ? `+${(ms / 1000).toFixed(2)} s` : `+${ms} ms`;
}
