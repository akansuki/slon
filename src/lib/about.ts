/**
 * The "what am I running" text. Kept pure so it can be tested, and deliberately free of
 * anything that identifies a server: this text is meant to be pasted into a public bug
 * report, so host names, user names and database names stay out of it.
 */
import type { BuildInfo } from './types';

const OS_NAMES: Record<string, string> = {
  windows: 'Windows',
  macos: 'macOS',
  linux: 'Linux',
};

export function osLabel(info: BuildInfo): string {
  return `${OS_NAMES[info.os] ?? info.os} ${info.arch}`;
}

/** "Slon 0.1.0", with a marker when it is not a release build. */
export function versionLabel(info: BuildInfo | null): string {
  if (!info) return 'Slon';
  return `Slon ${info.version}${info.debug ? ' (debug)' : ''}`;
}

/** The server version without the platform tail PostgreSQL appends. */
export function serverLabel(serverVersion: string | undefined): string | null {
  const first = serverVersion?.trim().split(/\s+/)[0];
  return first ? `PostgreSQL ${first}` : null;
}

/** The lines copied by the footer, newest-to-least useful. */
export function diagnostics(info: BuildInfo | null, serverVersion?: string): string {
  const lines = [versionLabel(info)];
  if (info) lines.push(osLabel(info));
  const server = serverLabel(serverVersion);
  if (server) lines.push(server);
  return lines.join('\n');
}
